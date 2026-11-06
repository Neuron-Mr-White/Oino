/** Explicitly illustrative v3 capture fixture, using the production event reducer. */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
const session = genSession('autonomy', { seed: 8, turns: 2, toolsPerTurn: 0, cwd: '/home/you/projects/unipi' });
const entries = session.snapshot.entries as Array<{ id: string; type: string; message?: { role?: string } }>;
const anchor = entries.find((e) => e.message?.role === 'user')!;
const last = entries.at(-1)!;
session.snapshot.entries = [...entries, { type: 'compaction_state', id: 'cutoff', parentId: last.id, timestamp: new Date().toISOString(), summaryText: 'Previous work summarized. Original messages remain available above.', summaryTokens: 20, summaryKind: 'llm_summary', anchorMessage: { id: anchor.id, index: 0 }, removedCount: 0, retainedSkills: [], retainedImages: [], loadedDeferredToolNames: [] }] as never;
session.snapshot.leaf_id = 'cutoff';
session.snapshot.commands = [{ name: 'plan', description: 'Plan mode', source: 'extension' }] as never;
session.snapshot.state = { ...session.snapshot.state as object, autonomy: 'medium' };
session.events = [{ at: 0, dto: { session: 'autonomy', seq: 1, event: { type: 'approval_request', requestId: 'approval-demo', sessionId: 'autonomy', tool: 'bash', arguments: { command: 'git push origin feature/autonomy', summary: 'Push the reviewed branch' }, riskLevel: 'high', ceiling: 'medium', reason: 'This command needs Autonomy High or your approval.' } } }];
session.cmd = { set_autonomy: () => undefined, approval_response: () => undefined };
export const autonomy: MockScenario = {
  name: 'autonomy', hosts: [{ id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, sessions: { autonomy: session } }],
  setup() { location.hash = '#/s/pc-coffee/autonomy'; },
};
