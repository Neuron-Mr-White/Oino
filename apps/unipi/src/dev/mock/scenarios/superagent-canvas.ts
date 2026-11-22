/**
 * The agent canvas (OIN-77/79): every role and all five states. The Superagent with three
 * earlier router chats; four board projects: ExampleApp (a stalled lead), Oino (a working graph
 * lead with nested subagents, graph workers, five ended siblings that fold, and chats), Orchard
 * (an idle, wakeable lead) and Website (a lead waiting on the owner); a folder without a board
 * ("notes") and "Other chats".
 *
 * `?scenario=superagent-canvas` with
 *   `&fleet=0|1|3|300|old`  0 = no agents, 1 = only Oino, 3 = all (default), 300 = stress, old = today's older-host payload
 *   `&canvas=1`             open the canvas on load
 *   `&hang=fleet`           every fleet call after the first never answers (stale chip, no overlapping polls)
 *   `&fleetError=1`         the first fleet call fails (first-load error banner)
 *   `&leadAction=unsupported` `superagent.lead_action` answers "unknown method" (the inline error on the health card)
 * `superagent.lead_action` is stateful: retry/wake start the lead, stop ends it, switch_model sets the model.
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
type Role = 'superagent' | 'lead' | 'worker' | 'subagent' | 'chat';
type Status = 'working' | 'waiting' | 'stalled' | 'idle' | 'ended';
export type MockNode = {
  id: string; parentId: string | null; project: string | null; projectId: string | null; groupKey: string | null; title: string; mode: 'plain' | 'graph' | 'swarm'; role: Role; status: Status;
  model: string | null; name: string | null; cwd: string | null; ticketIds: string[]; step: string | null; tool: string | null; question: { taskId: string; excerpt: string } | null;
  health: { error: string; attempts: number; since: number } | null; lastActiveAt: number | null; idleReason: 'unloaded' | null; startedAt: number | null; endedAt: number | null;
};
const ago = (min: number) => Date.now() - min * 60_000;
const HAIKU = 'anthropic/claude-haiku-4-5'; const SONNET = 'anthropic/claude-sonnet-4-5'; const GPT = 'openai/gpt-5';
const GROUPS = [
  { key: 'example-app', name: 'ExampleApp', hasBoard: true }, { key: 'oino', name: 'Oino', hasBoard: true }, { key: 'orchard', name: 'Orchard', hasBoard: true }, { key: 'website', name: 'Website', hasBoard: true },
  { key: 'folder:notes', name: 'notes', hasBoard: false }, { key: 'other', name: 'Other chats', hasBoard: false },
];
const P = { oino: { project: 'Oino', projectId: 'oino', groupKey: 'oino' }, web: { project: 'Website', projectId: 'website', groupKey: 'website' }, hp: { project: 'ExampleApp', projectId: 'example-app', groupKey: 'example-app' }, orchard: { project: 'Orchard', projectId: 'orchard', groupKey: 'orchard' },
  notes: { project: null, projectId: null, groupKey: 'folder:notes' }, other: { project: null, projectId: null, groupKey: 'other' }, router: { project: null, projectId: null, groupKey: null } } as const;

const node = (n: Partial<MockNode> & Pick<MockNode, 'id' | 'title' | 'status'>, min: number): MockNode => ({
  parentId: null, project: null, projectId: null, groupKey: null, mode: 'plain', role: 'subagent', model: HAIKU, name: null, cwd: null, ticketIds: [], step: null, tool: null, question: null, health: null, lastActiveAt: null, idleReason: null,
  startedAt: ago(min), endedAt: n.status === 'ended' ? ago(Math.max(1, min - 6)) : null, ...n,
});
const sub = (id: string, parentId: string, group: keyof typeof P, name: string, status: Status, min: number, extra: Partial<MockNode> = {}) => node({ id, parentId, ...P[group], title: name, name, status, ...extra }, min);

const router = [
  node({ id: 'superagent-chat', ...P.router, title: 'Router', role: 'superagent', status: 'idle', model: GPT, lastActiveAt: ago(1) }, 240),
  ...['Plan the release', 'Triage the inbox', 'Weekly review'].map((title, i) => node({ id: `rc-${i}`, ...P.router, title, role: 'chat', status: 'ended', model: GPT }, 3000 - i * 500)),
];
const oino: MockNode[] = [
  node({ id: 'lead-oino', ...P.oino, title: 'Lead', role: 'lead', mode: 'graph', status: 'working', model: GPT, ticketIds: ['OIN-14', 'OIN-16', 'OIN-18', 'OIN-19'], step: 'Running the workspace tests', tool: 'bash' }, 47),
  sub('sub-oino-core', 'lead-oino', 'oino', 'explore', 'working', 22, { step: 'Reading oino-core', tool: 'read' }),
  sub('sub-oino-migrate', 'lead-oino', 'oino', 'general', 'working', 18, { model: SONNET, step: 'Writing the migration', tool: 'edit' }),
  sub('sub-oino-tests', 'sub-oino-migrate', 'oino', 'test-runner', 'working', 6, { step: 'cargo test --workspace', tool: 'bash' }),
  sub('sub-oino-review', 'sub-oino-migrate', 'oino', 'reviewer', 'stalled', 9, { health: { error: 'Provider answered 429: rate limited', attempts: 3, since: ago(4) }, lastActiveAt: ago(4) }),
  sub('sub-oino-docs', 'lead-oino', 'oino', 'docs', 'idle', 12, { lastActiveAt: ago(8) }),
  ...['wk-parse', 'wk-render', 'wk-ship'].map((id, i) => node({ id, parentId: 'lead-oino', ...P.oino, title: ['Parse the config', 'Render the report', 'Ship the bundle'][i]!, name: ['parse', 'render', 'ship'][i]!, role: 'worker', mode: 'graph', status: (['working', 'working', 'idle'] as const)[i]!, step: i < 2 ? ['Resolving imports', 'Laying out tables'][i]! : null, tool: i < 2 ? 'edit' : null }, 14 - i)),
  ...['scout', 'explore', 'explore', 'scout', 'explore'].map((name, i) => sub(`sub-oino-old-${i}`, 'lead-oino', 'oino', name, 'ended', 44 - i * 3)),
  node({ id: 'chat-oino-auth', ...P.oino, title: 'Fix the flaky auth test', role: 'chat', status: 'idle', model: SONNET, cwd: '/home/you/oino', lastActiveAt: ago(35) }, 120),
  node({ id: 'chat-oino-rename', ...P.oino, title: 'Rename the session module', role: 'chat', status: 'working', model: GPT, cwd: '/home/you/oino', step: 'Updating imports', tool: 'edit' }, 9),
  sub('sub-chat-rename', 'chat-oino-rename', 'oino', 'explore', 'working', 3, { step: 'Searching for usages', tool: 'grep' }),
  node({ id: 'chat-oino-pr', ...P.oino, title: 'Review PR 88', role: 'chat', status: 'ended', model: SONNET, cwd: '/home/you/oino' }, 300),
];
const web: MockNode[] = [
  node({ id: 'lead-web', ...P.web, title: 'Lead', role: 'lead', mode: 'swarm', status: 'waiting', model: SONNET, ticketIds: ['WEB-8', 'WEB-9'], question: { taskId: 'WEB-8', excerpt: 'Which domain should the staging deploy use: staging.example.com or preview.example.com?' } }, 31),
  sub('sub-web-pricing', 'lead-web', 'web', 'copywriter', 'working', 14, { step: 'Pricing table copy', tool: 'write' }),
  sub('sub-web-deploy', 'lead-web', 'web', 'deployer', 'idle', 11, { model: 'openai/gpt-5-mini', lastActiveAt: ago(2) }),
  node({ id: 'chat-web-seo', ...P.web, title: 'Audit the SEO tags', role: 'chat', status: 'ended', model: SONNET, cwd: '/home/you/website' }, 500),
];
const hp: MockNode[] = [
  node({ id: 'lead-hp', ...P.hp, title: 'Lead', role: 'lead', status: 'stalled', model: SONNET, ticketIds: ['HP-1', 'HP-4'], health: { error: 'Authentication failed: invalid API key (401)', attempts: 3, since: ago(6) }, lastActiveAt: ago(6) }, 58),
  sub('sub-hp-sync', 'lead-hp', 'hp', 'explore', 'ended', 52),
];
const orchard: MockNode[] = [node({ id: 'lead-orchard', ...P.orchard, title: 'Lead', role: 'lead', status: 'idle', model: GPT, idleReason: 'unloaded', ticketIds: ['ORC-2'], lastActiveAt: ago(180) }, 400)];
const loose: MockNode[] = [
  node({ id: 'chat-notes-1', ...P.notes, title: 'Summarise the reading list', role: 'chat', status: 'working', model: SONNET, cwd: '/home/you/notes', step: 'Reading', tool: 'read' }, 6),
  node({ id: 'chat-notes-2', ...P.notes, title: 'Draft the newsletter', role: 'chat', status: 'ended', model: SONNET, cwd: '/home/you/notes' }, 700),
  node({ id: 'chat-other-1', ...P.other, title: 'Convert the CSV to JSON', role: 'chat', status: 'ended', model: HAIKU }, 900),
  node({ id: 'chat-other-2', ...P.other, title: 'Explain this stack trace', role: 'chat', status: 'idle', model: GPT, lastActiveAt: ago(60) }, 90),
];

/** The older host's payload: role lead for everything, no groups, no rev, no subagents. */
const olderHost = (): MockNode[] => [...oino.filter((n) => n.role === 'lead'), ...web.filter((n) => n.role === 'lead'), ...hp.filter((n) => n.role === 'lead'), ...loose.slice(2)]
  .map((n) => ({ ...n, role: 'lead' as const, groupKey: null, title: n.projectId ? `Lead · ${n.project}` : n.title, parentId: null, status: n.status === 'stalled' ? 'working' as const : n.status === 'waiting' ? 'working' as const : n.status }));

/** 300 nodes: 12 projects, each a lead with 23 subagents, nested a level. */
function stress(): MockNode[] {
  const out: MockNode[] = [...router.slice(0, 2)];
  const names = ['explore', 'general', 'test-runner', 'reviewer', 'docs', 'scout'];
  const statuses: Status[] = ['working', 'working', 'idle', 'ended', 'ended', 'waiting', 'ended', 'working'];
  GROUPS_STRESS.forEach((g, gi) => {
    out.push(node({ id: `s-lead-${g.key}`, project: g.name, projectId: g.key, groupKey: g.key, title: 'Lead', role: 'lead', mode: gi % 3 === 0 ? 'graph' : gi % 3 === 1 ? 'swarm' : 'plain', status: gi === 4 ? 'stalled' : gi === 5 ? 'waiting' : 'working', ticketIds: [`${g.name.slice(0, 3).toUpperCase()}-1`], health: gi === 4 ? { error: 'Provider answered 500', attempts: 2, since: ago(3) } : null, question: gi === 5 ? { taskId: 'X-1', excerpt: 'Which branch?' } : null, step: 'Working', tool: 'bash' }, 60 + gi));
    for (let i = 0; i < 22; i++) {
      const parent = i >= 14 ? `s-${g.key}-${i - 14}` : `s-lead-${g.key}`;
      out.push(node({ id: `s-${g.key}-${i}`, parentId: parent, project: g.name, projectId: g.key, groupKey: g.key, title: names[i % names.length]!, name: names[i % names.length]!, status: statuses[(i + gi) % statuses.length]!, step: 'Working', tool: 'bash' }, 50 - (i % 40)));
    }
    out.push(node({ id: `s-chat-${g.key}`, project: g.name, projectId: g.key, groupKey: g.key, title: `Chat in ${g.name}`, role: 'chat', status: 'idle', model: SONNET }, 200));
  });
  return out;
}
const GROUPS_STRESS = Array.from({ length: 12 }, (_, i) => ({ key: `p${String(i + 1).padStart(2, '0')}`, name: `Project ${String(i + 1).padStart(2, '0')}`, hasBoard: true }));

const hashOf = (nodes: MockNode[]) => {
  const text = nodes.map((n) => [n.id, n.parentId, n.role, n.status, n.groupKey, n.title, n.name, n.model, n.question?.excerpt, n.health?.error, n.idleReason].join('|')).join('\n');
  let h = 5381;
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) | 0;
  return (h >>> 0).toString(16);
};
const STEPS = ['Reading the code', 'Running the tests', 'Editing the module', 'Checking the diff', 'Searching for usages'];

export function canvasScenario(count: number | 'old' | 'stress' = 3, opts: { hang?: boolean; fleetError?: boolean; leadActionUnsupported?: boolean } = {}): MockScenario {
  const stressMode = count === 300 || count === 'stress';
  let nodes: MockNode[] = count === 0 ? [] : count === 'old' ? olderHost() : count === 1 ? [...router, ...oino] : stressMode ? stress() : [...router, ...hp, ...oino, ...orchard, ...web, ...loose];
  const groups = count === 0 ? [] : count === 'old' ? [] : count === 1 ? GROUPS.filter((g) => g.key === 'oino') : stressMode ? GROUPS_STRESS : GROUPS;
  let calls = 0; let tick = 0;
  const edges = () => nodes.filter((n) => n.parentId && nodes.some((p) => p.id === n.parentId)).map((n) => ({ from: n.parentId, to: n.id }));
  const setNode = (id: string, patch: Partial<MockNode>) => { nodes = nodes.map((n) => (n.id === id ? { ...n, ...patch } : n)); };
  const live = () => nodes.map((n, i) => (n.status === 'working' ? { ...n, step: STEPS[(tick + i) % STEPS.length]!, tool: n.tool ?? 'bash' } : n));
  const leads = () => nodes.filter((n) => n.role === 'lead' && n.status !== 'idle' && n.status !== 'ended' && n.projectId).map((n) => ({
    projectId: n.projectId!, sessionId: n.id, presetId: null, model: n.model, thinking: 'high', autonomy: 'high', mode: n.mode === 'plain' ? 'none' : n.mode,
    ticketIds: n.ticketIds, step: n.step, tool: n.tool, startedAt: new Date(n.startedAt!).toISOString(), cost: 0.5, progress: null,
  }));
  const rows = () => nodes.filter((n) => n.role === 'lead' || n.role === 'superagent' || (n.role === 'chat' && !n.groupKey)).map((n) => ({ sessionId: n.id, title: n.title, project: n.project, createdAt: n.startedAt!, updatedAt: n.startedAt!, path: `/mock/${n.id}`, state: { kind: n.status === 'working' ? 'working' : 'idle' } }));
  // A subagent's own transcript, read through its owner's `_oino/agent_messages` (what the peek shows for a child).
  const childMessages = (agentId: string) => {
    const n = nodes.find((x) => x.id === agentId);
    const text = (t: string) => ({ type: 'text', text: t });
    return [
      { role: 'user', content: [text(`Task for ${n?.name ?? 'the subagent'}: ${n?.step ?? 'look into the reducer and report back.'}`)] },
      { role: 'assistant', content: [text('Reading the reducer and the state type.'), { type: 'toolCall', id: 'c1', name: 'read', arguments: { path: 'src/lib/session/reducer.ts' } }] },
      { role: 'toolResult', toolCallId: 'c1', toolName: 'read', content: [text('export type SessionState = { seq, entries, run, queue, topics }')] },
      { role: 'assistant', content: [text('The reducer owns **entries**, **run**, **queue** and **topics**; the turns index is derived.'), { type: 'toolCall', id: 'c2', name: 'grep', arguments: { pattern: 'export function apply', path: 'src/lib/session' } }] },
      { role: 'toolResult', toolCallId: 'c2', toolName: 'grep', content: [text('reducer.ts: 4 hits\napplySnapshot, applyEvents, initial')] },
    ];
  };
  const withChildren = (session: ReturnType<typeof genSession>) => ({
    ...session,
    cmd: {
      ...(session.cmd ?? {}),
      '_oino/agents': () => ({ agents: nodes.filter((n) => n.role === 'subagent' || n.role === 'worker').map((n) => ({ id: n.id, label: `Subagent · ${n.name ?? n.title}`, owner: 'unipi.subagents', role: 'subagent_explore', state: n.status === 'working' ? 'running' : 'done', depth: 1 })) }),
      '_oino/agent_messages': (c: { type: string }) => ({ messages: childMessages(String((c as { agentId?: string }).agentId ?? '')) }),
    },
  });
  const sessions = Object.fromEntries(['lead-web', 'lead-orchard', 'lead-hp', 'lead-oino', 'chat-oino-rename', 'chat-notes-1'].map((id) => [id, withChildren(genSession(id, { seed: 9, turns: 4, toolsPerTurn: 2, cwd: `/home/you/projects/${id.split('-')[1]}` }))]));
  const fail = (code: string, message: string) => ({ code, message, retryable: false });
  return {
    name: 'superagent-canvas',
    hosts: [{
      ...base,
      sessions: { ...base.sessions, ...sessions },
      ops: {
        ...base.ops,
        'superagent.fleet': async () => {
          calls++; tick++;
          if (opts.fleetError && calls === 1) throw fail('E_TRANSPORT', 'The host could not list agents');
          if (opts.hang && calls > 1) return new Promise(() => {});
          const modern = count !== 'old';
          return { nodes: live(), edges: edges(), ...(modern ? { superagentId: nodes.some((n) => n.id === 'superagent-chat') ? 'superagent-chat' : null, groups, rev: hashOf(nodes) } : {}) };
        },
        'superagent.lead_action': (args) => {
          if (opts.leadActionUnsupported) throw fail('E_SCHEMA', 'unknown variant `superagent.lead_action`');
          const id = String(args.sessionId); const action = String(args.action);
          if (!nodes.some((n) => n.id === id)) throw fail('E_NOT_FOUND', `No lead ${id}`);
          if (action === 'retry' || action === 'wake') setNode(id, { status: 'working', health: null, idleReason: null, step: action === 'retry' ? 'Retrying' : 'Waking up', endedAt: null });
          else if (action === 'switch_model') setNode(id, { status: 'working', health: null, model: String(args.model ?? GPT), step: 'Restarted on the new model' });
          else if (action === 'stop') setNode(id, { status: 'ended', endedAt: Date.now(), health: null, question: null });
          else throw fail('E_INVALID', `Unknown action ${action}`);
          return { ok: true };
        },
        'superagent.status': (args, host) => {
          const status = base.ops!['superagent.status']!(args, host) as { needsYou: unknown[]; review: unknown[] };
          const questions = nodes.filter((n) => n.status === 'waiting' && n.question && n.projectId).map((n) => ({ projectId: n.projectId!, taskId: n.question!.taskId, status: 'blocked', question: n.question!.excerpt }));
          return { ...status, leads: leads(), needsYou: count === 0 ? [] : questions, review: count === 0 ? [] : status.review };
        },
        'superagent.sessions': () => [...rows(), { sessionId: 'old-chat', title: 'Previous investigation', project: 'Website', createdAt: ago(1500), updatedAt: ago(1400), path: '/mock/old', state: { kind: 'idle' } }],
      },
    }],
    setup() { location.hash = `#/superagent?session=superagent-chat${new URLSearchParams(location.search).get('canvas') === '1' ? '&canvas=1' : ''}`; },
  };
}

/** The capture URL carries its options in the hash (`#/superagent?scenario=...&fleet=300`) or in the search string. */
const params = typeof location === 'undefined' ? new URLSearchParams() : new URLSearchParams(location.hash.includes('?') ? location.hash.slice(location.hash.indexOf('?') + 1) : location.search);
const raw = params.get('fleet') ?? '3';
export const superagentCanvas: MockScenario = canvasScenario(raw === 'old' ? 'old' : Number(raw), { hang: params.get('hang') === 'fleet', fleetError: params.get('fleetError') === '1', leadActionUnsupported: params.get('leadAction') === 'unsupported' });
