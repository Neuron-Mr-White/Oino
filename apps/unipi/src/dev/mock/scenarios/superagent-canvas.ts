/**
 * The agent canvas (OIN-50): three projects with a lead each (Oino in Graph mode with
 * subagents, Website in Swarm mode with a waiting subagent, Heartpick Plain and alone),
 * an idle lead, an ended subagent and one orphan with no project.
 * `?scenario=superagent-canvas&fleet=0|1|3` (0 = no agents, 1 = only Oino, 3 = all) and
 * `&canvas=1` to open the canvas on load.
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
type Node = { id: string; parentId: string | null; project: string | null; projectId: string | null; title: string; mode: 'plain' | 'graph' | 'swarm'; role: 'lead' | 'worker' | 'subagent'; status: 'working' | 'waiting' | 'idle' | 'ended'; model: string | null; startedAt: number | null; endedAt: number | null };
const ago = (min: number) => Date.now() - min * 60_000;
const OINO = { project: 'Oino', projectId: 'oino' };
const WEB = { project: 'Website', projectId: 'website' };
const node = (n: Partial<Node> & Pick<Node, 'id' | 'title' | 'status'>, min: number): Node => ({ parentId: null, project: null, projectId: null, mode: 'plain', role: 'subagent', model: 'anthropic/claude-haiku-4-5', startedAt: ago(min), endedAt: n.status === 'ended' ? ago(min - 6) : null, ...n });

const oino: Node[] = [
  node({ id: 'lead-oino', ...OINO, title: 'Lead · Oino', role: 'lead', mode: 'graph', status: 'working', model: 'openai/gpt-5' }, 47),
  node({ id: 'sub-oino-scan', parentId: 'lead-oino', ...OINO, title: 'Scan the repo', status: 'ended' }, 40),
  node({ id: 'sub-oino-core', parentId: 'lead-oino', ...OINO, title: 'Explore oino-core', status: 'working' }, 22),
  node({ id: 'sub-oino-migrate', parentId: 'lead-oino', ...OINO, title: 'Write the migration', status: 'working', model: 'anthropic/claude-sonnet-4-5' }, 18),
  node({ id: 'sub-oino-tests', parentId: 'sub-oino-migrate', ...OINO, title: 'Run the workspace tests', status: 'working' }, 6),
  node({ id: 'sub-oino-review', parentId: 'sub-oino-migrate', ...OINO, title: 'Review the diff', status: 'idle' }, 9),
  node({ id: 'sub-oino-docs', parentId: 'lead-oino', ...OINO, title: 'Update the docs', status: 'idle' }, 12),
];
const rest: Node[] = [
  node({ id: 'lead-web', ...WEB, title: 'Lead · Website', role: 'lead', mode: 'swarm', status: 'working', model: 'anthropic/claude-sonnet-4-5' }, 31),
  node({ id: 'sub-web-pricing', parentId: 'lead-web', ...WEB, title: 'Pricing table copy', status: 'working' }, 14),
  node({ id: 'sub-web-deploy', parentId: 'lead-web', ...WEB, title: 'Staging deployment', status: 'waiting', model: 'openai/gpt-5-mini' }, 11),
  node({ id: 'lead-hp', project: 'Heartpick', projectId: 'heartpick', title: 'Lead · Heartpick', role: 'lead', status: 'working', model: 'anthropic/claude-sonnet-4-5' }, 18),
  node({ id: 'lead-orchard', project: 'Orchard', projectId: 'orchard', title: 'Lead · Orchard', role: 'lead', status: 'idle', model: 'openai/gpt-5' }, 95),
  node({ id: 'sub-orphan', parentId: 'gone-session', title: 'Check the changelog format', status: 'working' }, 4),
];

export function canvasScenario(count: number): MockScenario {
  const nodes = count <= 0 ? [] : count === 1 ? oino : [...oino, ...rest];
  const leads = nodes.filter((n) => n.role === 'lead' && n.status !== 'idle' && n.projectId).map((n) => ({
    projectId: n.projectId!, sessionId: n.id, presetId: null, model: n.model, thinking: 'high', autonomy: 'high', mode: n.mode === 'plain' ? 'none' : n.mode,
    ticketIds: [], step: n.mode === 'graph' ? 'Running the workspace tests' : 'Working', tool: 'bash', startedAt: new Date(n.startedAt!).toISOString(), cost: 0.5, progress: null,
  }));
  const rows = nodes.filter((n) => n.role === 'lead' && n.projectId).map((n) => ({ sessionId: n.id, title: n.title, project: n.project, createdAt: n.startedAt!, updatedAt: n.startedAt!, path: `/mock/${n.id}`, state: { kind: n.status === 'working' ? 'working' : 'idle' } }));
  const sessions = Object.fromEntries(['lead-web', 'lead-orchard'].map((id) => [id, genSession(id, { seed: 9, turns: 1, toolsPerTurn: 1, cwd: `/home/you/projects/${id.slice(5)}` })]));
  return {
    name: 'superagent-canvas',
    hosts: [{
      ...base,
      sessions: { ...base.sessions, ...sessions },
      ops: {
        ...base.ops,
        'superagent.fleet': () => ({ nodes, edges: nodes.filter((n) => n.parentId && nodes.some((p) => p.id === n.parentId)).map((n) => ({ from: n.parentId, to: n.id })) }),
        'superagent.status': (args, host) => {
          const status = base.ops!['superagent.status']!(args, host) as { needsYou: unknown[]; review: unknown[] };
          return { ...status, leads, needsYou: count > 0 ? status.needsYou : [], review: count > 0 ? status.review : [] };
        },
        'superagent.sessions': () => [...rows, { sessionId: 'old-chat', title: 'Previous investigation', project: 'Website', createdAt: ago(1500), updatedAt: ago(1400), path: '/mock/old', state: { kind: 'idle' } }],
      },
    }],
    setup() { location.hash = `#/superagent?session=superagent-chat${new URLSearchParams(location.search).get('canvas') === '1' ? '&canvas=1' : ''}`; },
  };
}

const count = typeof location === 'undefined' ? 3 : Number(new URLSearchParams(location.search).get('fleet') ?? 3);
export const superagentCanvas: MockScenario = canvasScenario(count);
