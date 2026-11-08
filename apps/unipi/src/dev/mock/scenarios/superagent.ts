/** Illustrative Superagent capture, with real session reducer and shared UI. */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
const chat = genSession('superagent-chat', { seed: 14, turns: 1, toolsPerTurn: 0, cwd: '' });
chat.snapshot.state = { ...chat.snapshot.state as object, cwd: '' };
chat.snapshot.entries = [
  { type: 'custom', id: 'sa-kind', parentId: null, customType: 'oino:superagent', data: { workspace: null } },
  { type: 'message', id: 'sa-user', parentId: 'sa-kind', message: { role: 'user', content: 'What is running, and what needs my attention?' } },
  { type: 'message', id: 'sa-answer', parentId: 'sa-user', message: { role: 'assistant', content: [{ type: 'text', text: 'Oino is working on **OIN-14**, the Superagent interface. The worker is running the focused UI tests.\n\n**Two tickets need you:**\n- **OIN-12** is ready for review.\n- **WEB-8** is blocked on a deployment decision.\n\nOpen a ticket to review or answer its question. To follow up with a running worker, use Relay.' }], stopReason: 'stop' } },
] as never;
chat.snapshot.leaf_id = 'sa-answer';
// Capture variants keep the ordinary demo unchanged.
const capture = typeof location === 'undefined' ? '' : new URLSearchParams(location.search).get('saCapture');
if (capture === 'empty') { chat.snapshot.entries = []; chat.snapshot.leaf_id = null; }
const worker = genSession('worker-14', { seed: 3, turns: 1, toolsPerTurn: 1, cwd: '/home/you/projects/oino' });
let enabled = true;
let concurrencyCap = 2;
let stopped = false;
export const superagent: MockScenario = {
  name: 'superagent',
  hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, newSessionFrom: 'superagent-chat', sessions: { 'superagent-chat': chat, 'worker-14': worker }, ops: {
    'superagent.sessions': () => [
      { sessionId: 'superagent-chat', title: 'Superagent conversation', createdAt: 1794045600000, updatedAt: 1794045600000, path: '/mock/superagent-chat' },
      { sessionId: 'superagent-triage', title: 'What is running and what needs my attention?', createdAt: 1793978400000, updatedAt: 1793978400000, path: '/mock/superagent-triage' },
      { sessionId: 'superagent-review', title: 'Start OIN-14 and tell me what needs review', createdAt: 1793869920000, updatedAt: 1793869920000, path: '/mock/superagent-review' },
      { sessionId: 'superagent-old', title: 'Superagent · 2026-11-08 09:03', createdAt: 1794128580000, updatedAt: 1794128580000, path: '/mock/superagent-old' },
    ],
    'superagent.status': () => ({ enabled, concurrencyCap, workers: stopped ? [] : [{ projectId: 'oino', taskId: 'OIN-14', sessionId: 'worker-14', autonomy: 'medium', step: 'Running focused UI tests', tool: 'bash', startedAt: new Date(Date.now() - 420000).toISOString(), cost: 0.42 }], needsYou: [...(capture === 'approval' ? [{ projectId: 'oino', taskId: 'OIN-14', status: 'blocked', workerSessionId: 'worker-14', requestId: 'demo-approval', sessionId: 'worker-14', tool: 'bash', command: 'git push origin feature', summary: 'Push the feature branch', risk: 'high', ceiling: 'medium', reason: 'needs Autonomy High' }] : []), { projectId: 'oino', taskId: 'OIN-12', status: 'in_review', question: null }, { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Should the staging deployment use the existing domain, or should I create a separate preview domain?' }] }),
    'superagent.configure': (args) => { enabled = args.enabled as boolean; concurrencyCap = args.concurrencyCap as number; return { enabled, concurrencyCap }; },
    'superagent.stop': () => { stopped = true; return { stopped: true }; },
    'superagent.start': () => { stopped = false; return { sessionId: 'worker-14' }; },
    'superagent.relay': () => ({ delivered: true, commentId: 'comment-demo', sessionId: 'worker-14' }),
    board_cli: (args) => {
      const id = (args.args as string[])[1];
      return { ok: true, payload: { id, title: id === 'OIN-14' ? 'Superagent interface' : id === 'OIN-12' ? 'Autonomy controls and approvals' : 'Staging deployment', autonomy: id === 'WEB-8' ? 'high' : 'medium', status: id === 'OIN-14' ? 'in_progress' : id === 'OIN-12' ? 'in_review' : 'blocked', priority: 'medium', order: 0, deps: [], labels: [], created: '', updated: '', activity: [] } };
    },
  } }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
