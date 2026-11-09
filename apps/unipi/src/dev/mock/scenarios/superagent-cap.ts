/** Owner's full worker-cap case: two leads occupy both slots, cloud-agent has eight ready tickets. */
import type { MockScenario } from '../host';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
export const superagentCap: MockScenario = {
  name: 'superagent-cap',
  hosts: [{ ...base, ops: {
    ...base.ops,
    'superagent.status': () => ({
      enabled: true, concurrencyCap: 2,
      leads: [
        { projectId: 'oino', sessionId: 'lead-oino', autonomy: 'high', ticketIds: ['OIN-48'], startedAt: new Date(Date.now() - 47 * 60000).toISOString() },
        { projectId: 'zmstudio-migration', sessionId: 'lead-zm', autonomy: 'high', ticketIds: ['ZM-1'], startedAt: new Date(Date.now() - 18 * 60000).toISOString() },
      ], needsYou: [], review: [],
      projects: [
        { projectId: 'cloud-agent', name: 'cloud-agent', autoStart: true, ready: 8, lead: null },
        { projectId: 'oino', name: 'oino', autoStart: true, ready: 0, lead: 'lead-oino' },
        { projectId: 'zmstudio-migration', name: 'zmstudio-migration', autoStart: true, ready: 0, lead: 'lead-zm' },
      ],
    }),
    'superagent.start': () => { throw { code: 'E_WORKER_CAP', message: 'Worker cap reached; wait for a worker or raise concurrencyCap', retryable: false }; },
    board_cli: (args) => {
      const [cmd, id] = args.args as string[];
      const task = (taskId: string, order: number) => ({ id: taskId, title: `Cloud task ${order + 1}`, status: 'todo', priority: 'medium', order, deps: [], labels: [], created: '', updated: '', activity: [], ready: true });
      return cmd === 'list' ? { ok: true, payload: { problems: [], tasks: Array.from({ length: 8 }, (_, i) => task(`CLD-${i + 1}`, i)) } } : { ok: true, payload: task(id ?? 'CLD-1', 0) };
    },
  } }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
