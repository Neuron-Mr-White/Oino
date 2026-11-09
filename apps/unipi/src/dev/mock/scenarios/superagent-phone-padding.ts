/**
 * OIN-64 repro: a Superagent lead transcript with a running group of four
 * subagent cards (the "Running 4 subagents" container), for checking the
 * phone gutters against the composer's. `?scenario=superagent-phone-padding`.
 */
import type { MockScenario } from '../host';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
const chat = structuredClone(base.sessions!['superagent-chat']!);
const titles: Array<[string, string]> = [
  ['Provision sandbox', 'c1'],
  ['Inventory apps', 'c2'],
  ['Find Stripe config', 'c3'],
  ['Map the order tables', 'c4'],
];
const step = (id: string, agentId: string, title: string, name: string, arg: string) => ({
  type: 'custom', id, parentId: 'sa-calls', customType: 'subagent-step',
  data: { kind: 'tool', name, arg, args: name === 'bash' ? { command: arg } : { path: arg }, output: 'ok', isError: false, durationMs: 3, agentId, id: agentId, title, profile: 'explore' },
});
chat.snapshot.state = { ...chat.snapshot.state as object, isStreaming: true };
chat.snapshot.entries = [
  { type: 'custom', id: 'sa-kind', parentId: null, customType: 'oino:superagent', data: { workspace: null } },
  { type: 'message', id: 'sa-user', parentId: 'sa-kind', message: { role: 'user', content: 'Migrate the zmstudio store to the new schema.' } },
  { type: 'message', id: 'sa-calls', parentId: 'sa-user', message: { role: 'assistant', stopReason: 'toolUse', content: [
    { type: 'text', text: 'I am splitting the migration into four parallel investigations.' },
    ...titles.map(([title, id]) => ({ type: 'toolCall', id, name: 'run_subagent', arguments: { title, profile: 'explore', task: 'Investigate and report.' } })),
  ] } },
  step('st1', 'a1', 'Provision sandbox', 'bash', 'ssh box make build'),
  step('st2', 'a2', 'Inventory apps', 'read', '/p/apps/store/index.vue'),
  step('st3', 'a3', 'Find Stripe config', 'grep', 'STRIPE_KEY'),
  step('st4', 'a2', 'Inventory apps', 'read', '/p/apps/admin/index.vue'),
] as never;
chat.snapshot.leaf_id = 'st4';

export const superagentPhonePadding: MockScenario = {
  name: 'superagent-phone-padding',
  hosts: [{ ...base, sessions: { ...base.sessions, 'superagent-chat': chat } }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
