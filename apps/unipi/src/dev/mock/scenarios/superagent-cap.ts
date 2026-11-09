/** Three projects running concurrently, and one waiting because Auto is off. */
import type { MockScenario } from '../host';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
export const superagentCap: MockScenario = {
  name: 'superagent-cap',
  hosts: [{ ...base, ops: {
    ...base.ops,
    'superagent.status': () => ({
      enabled: true,
      leads: [
        { projectId: 'oino', sessionId: 'lead-oino', autonomy: 'high', ticketIds: ['OIN-48'], startedAt: new Date(Date.now() - 47 * 60000).toISOString() },
        { projectId: 'zmstudio-migration', sessionId: 'lead-zm', autonomy: 'high', ticketIds: ['ZM-1'], startedAt: new Date(Date.now() - 18 * 60000).toISOString() },
        { projectId: 'cloud-agent', sessionId: 'lead-cloud', autonomy: 'high', ticketIds: ['CLD-1'], startedAt: new Date(Date.now() - 6 * 60000).toISOString() },
      ], needsYou: [], review: [],
      projects: [
        { projectId: 'cloud-agent', name: 'cloud-agent', autoStart: true, ready: 0, lead: 'lead-cloud' },
        { projectId: 'waiting', name: 'waiting-project', autoStart: false, ready: 2, lead: null, blockers: [{ code: 'project_auto_off', message: 'Auto is off', action: { kind: 'enable_project_auto', label: 'Turn on Auto' } }] },
        { projectId: 'oino', name: 'oino', autoStart: true, ready: 0, lead: 'lead-oino' },
        { projectId: 'zmstudio-migration', name: 'zmstudio-migration', autoStart: true, ready: 0, lead: 'lead-zm' },
      ],
    }),
    board_cli: (args) => {
      const [cmd, id] = args.args as string[];
      const task = (taskId: string, order: number) => ({ id: taskId, title: `Cloud task ${order + 1}`, status: 'todo', priority: 'medium', order, deps: [], labels: [], created: '', updated: '', activity: [], ready: true });
      return cmd === 'list' ? { ok: true, payload: { problems: [], tasks: Array.from({ length: 8 }, (_, i) => task(`CLD-${i + 1}`, i)) } } : { ok: true, payload: task(id ?? 'CLD-1', 0) };
    },
  } }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
