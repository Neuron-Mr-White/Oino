/**
 * Long-horizon view scenarios ([lh-ui]) — the `long-horizon.graph` service (snapshot / history /
 * retry) answered from in-memory fixtures, so the real LH views run in the capture harness:
 *
 *   lh-graph-big   40 nodes, mixed statuses (running / errored / failed / superseded / queued …), with history
 *   lh-history     the same project with 3 graphs in History (one open, one finished, one stopped)
 *   lh-swarm       a swarm of 8 independent workers
 *   lh-goal        a goal mid-run;  lh-goal-errored  the same stopped on an error (Retry)
 *   lh-ralph       a ralph loop with a checklist
 *
 * Retry flips an errored node to running (event `continued` by user) like the real service.
 */
import type { MockScenario } from '../host';
import type { RpcCommand } from '../../../lib/host/client';
import { LH_EXTENSIONS, host, seedSessionRoute, topic } from './status';

const MIN = 60_000;
const NOW = Date.now();
const PROFILES = ['subagent_explore', 'subagent_general', 'rel-qa', 'subagent_review'];
const MODELS: Array<[string, string]> = [['zai/glm-5.3', 'high'], ['cc/claude-opus-5-5', 'medium'], ['zai/glm-5.3-flash', 'low'], ['openai/gpt-5.5', 'high']];
const VERBS = ['Survey', 'Port', 'Test', 'Review', 'Document', 'Refactor', 'Profile', 'Audit', 'Migrate', 'Wire'];
const AREAS = ['the lexer', 'the parser', 'the AST', 'the sidebar', 'the host RPC', 'the cache', 'settings', 'the timeline', 'the composer', 'phone sheets'];

type W = Record<string, unknown> & { workId: string; status: string; events: Array<Record<string, unknown>> };

type Spec = { id: string; status: string; inputs?: string[]; i: number; start: number; error?: string; supersededBy?: string };

function work(spec: Spec): W {
  const { id, status, inputs = [], i, start } = spec;
  const [model, thinking] = MODELS[i % MODELS.length]!;
  const ev = (ts: number, st: string, by: string, detail?: string) => ({ ts, status: st, by, ...(detail ? { detail } : {}) });
  const events = [ev(start, 'queued', 'lead')];
  const ran = status !== 'queued' && status !== 'blocked';
  if (ran) events.push(ev(start + 20_000, 'running', 'host', `operator op${i.toString(16).padStart(6, '0')}`));
  if (status === 'errored') {
    events.push(ev(start + 3 * MIN, 'continued', 'auto', 'auto-continue 1/1 in 30s after transient (network): connection error: reset by peer'));
    events.push(ev(start + 4 * MIN, 'running', 'auto', 'resumed operator'));
    events.push(ev(start + 6 * MIN, 'errored', 'host', spec.error ?? 'transient (5xx), auto-continue used: 503 Service Unavailable'));
  }
  if (status === 'completed') events.push(ev(start + (3 + (i % 5)) * MIN, 'completed', 'host'));
  if (status === 'failed') events.push(ev(start + 5 * MIN, 'failed', 'lead', 'the API is gone'));
  if (status === 'superseded') events.push(ev(start + 2 * MIN, 'superseded', 'lead', `replaced by ${spec.supersededBy}`));
  if (status === 'stopped') events.push(ev(start + 2 * MIN, 'stopped', 'user', 'cancelled'));
  return {
    workId: id,
    status,
    instruction: `${VERBS[i % VERBS.length]} ${AREAS[(i * 3) % AREAS.length]} and report what changed, with the files touched, the tests added and anything left for the next step to pick up.`,
    profile: PROFILES[i % PROFILES.length],
    inputIds: inputs.map((x) => `r-${x}`),
    attempts: ran ? 1 : 0,
    autoContinues: status === 'errored' ? 1 : 0,
    continues: 0,
    restartResumes: 0,
    toolCalls: ran ? 3 + ((i * 7) % 40) : 0,
    canRetry: status === 'errored',
    hasOutput: ran,
    events,
    eventsDropped: 0,
    ...(ran ? { model, thinking, operatorId: `op${i.toString(16).padStart(6, '0')}` } : {}),
    ...(status === 'completed' ? { resultId: `r-${id}` } : {}),
    ...(status === 'errored' ? { failureClass: 'transient', error: spec.error ?? 'transient (5xx), auto-continue used: 503 Service Unavailable' } : {}),
    ...(status === 'failed' ? { error: 'the API is gone', failureClass: 'lead' } : {}),
    ...(status === 'superseded' ? { supersededBy: spec.supersededBy } : {}),
    ...(status === 'blocked' ? { blockedBy: inputs[0] ?? 'an input' } : {}),
    ...(status === 'running' ? { cost: 0.0123 * (i + 1) } : {}),
  };
}

const KEYS = ['queued', 'running', 'blocked', 'errored', 'completed', 'failed', 'stopped', 'cancelled', 'superseded'] as const;
const countsOf = (list: readonly W[]) => Object.fromEntries(KEYS.map((k) => [k, list.filter((w) => w.status === k).length]));

type G = Record<string, unknown> & { graphId: string; work: W[]; events: Array<Record<string, unknown>>; state: string };

function graph(o: { id: string; mode: 'graph' | 'swarm'; task: string; state: 'open' | 'finished' | 'stopped'; createdAt: number; specs: Spec[]; events?: Array<Record<string, unknown>> }): G {
  const list = o.specs.map(work);
  const updatedAt = Math.max(o.createdAt, ...list.flatMap((w) => w.events.map((e) => e.ts as number)));
  return {
    v: 1,
    graphId: o.id,
    mode: o.mode,
    task: o.task,
    state: o.state,
    revision: 3,
    createdAt: o.createdAt,
    updatedAt,
    workCount: list.length,
    counts: countsOf(list),
    events: o.events ?? [
      { ts: o.createdAt, kind: 'opened', by: 'lead', detail: o.task },
      { ts: updatedAt - 30_000, kind: 'checkpoint', by: 'host', detail: `needs_attention: Agent graph "${o.task}" checkpoint: ${list.filter((w) => w.status === 'errored').length} errored.` },
      ...(o.state === 'finished' ? [{ ts: updatedAt, kind: 'finished', by: 'lead', detail: 'all steps done' }] : []),
      ...(o.state === 'stopped' ? [{ ts: updatedAt, kind: 'stopped', by: 'user', detail: 'stopped(user_requested)' }] : []),
    ],
    eventsDropped: 0,
    work: list,
    ...(o.state === 'finished' ? { finish: { resultIds: [], reason: 'all steps done', at: updatedAt } } : {}),
    ...(o.state === 'stopped' ? { abandoned: 'stopped(user_requested)' } : {}),
    omittedWork: 0,
  };
}

/** 40 nodes in 5 stages of 8: mixed statuses, one superseded chain, a blocked tail. */
function bigGraph(): G {
  const specs: Spec[] = [];
  const statuses = ['completed', 'completed', 'completed', 'completed', 'completed', 'completed', 'failed', 'superseded', 'completed', 'completed', 'completed', 'errored', 'completed', 'completed', 'completed', 'running', 'running', 'errored', 'completed', 'completed', 'running', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'blocked', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued', 'queued'];
  for (let i = 0; i < 40; i++) {
    const stage = Math.floor(i / 8);
    const inputs = stage === 0 ? [] : [`w${(stage - 1) * 8 + (i % 8)}`, ...(i % 3 === 0 ? [`w${(stage - 1) * 8 + ((i + 1) % 8)}`] : [])];
    specs.push({ id: `w${i}`, status: statuses[i]!, inputs, i, start: NOW - (60 - i) * MIN, ...(statuses[i] === 'superseded' ? { supersededBy: 'w8' } : {}) });
  }
  return graph({ id: 'graph-big-0001', mode: 'graph', task: 'Port the parser and the sidebar to the new host RPC', state: 'open', createdAt: NOW - 62 * MIN, specs });
}

function smallGraph(): G {
  const s = (id: string, status: string, i: number, inputs: string[] = [], extra: Partial<Spec> = {}): Spec => ({ id, status, i, inputs, start: NOW - (30 - i * 3) * MIN, ...extra });
  return graph({
    id: 'graph-77',
    mode: 'graph',
    task: 'Land the status pane sections (WP-A16)',
    state: 'open',
    createdAt: NOW - 34 * MIN,
    specs: [s('survey', 'completed', 0), s('topics', 'completed', 1, ['survey']), s('capsule', 'completed', 2, ['survey']), s('section', 'running', 3, ['topics']), s('phone', 'running', 4, ['topics']), s('docs', 'failed', 5, ['capsule']), s('ship', 'queued', 6, ['section', 'phone'])],
  });
}

function history(): G[] {
  const s = (id: string, status: string, i: number, inputs: string[] = [], extra: Partial<Spec> = {}): Spec => ({ id, status, i, inputs, start: NOW - 5 * 3600_000 + i * MIN, ...extra });
  return [
    graph({ id: 'graph-hist-2', mode: 'swarm', task: 'Audit the sidebar for phone overflow', state: 'finished', createdAt: NOW - 5 * 3600_000, specs: [s('a', 'completed', 0), s('b', 'completed', 1), s('c', 'completed', 2), s('d', 'completed', 3)] }),
    graph({ id: 'graph-hist-1', mode: 'graph', task: 'Migrate the settings pages to the write path', state: 'finished', createdAt: NOW - 26 * 3600_000, specs: [s('read', 'completed', 0), s('write', 'completed', 1, ['read']), s('tests', 'failed', 2, ['write']), s('fix', 'completed', 3, ['tests']), s('old', 'superseded', 4, [], { supersededBy: 'fix' })] }),
    graph({ id: 'graph-hist-0', mode: 'graph', task: 'Prototype the iroh relay fallback', state: 'stopped', createdAt: NOW - 3 * 24 * 3600_000, specs: [s('probe', 'completed', 0), s('relay', 'stopped', 1, ['probe']), s('bench', 'errored', 2, ['probe'])] }),
  ];
}

const headOf = (g: G) => {
  const { events: _e, eventsDropped: _d, work: _w, omittedWork: _o, ...head } = g;
  return head;
};

type World = { current: G | undefined; past: G[]; goal?: { errored: boolean } };

function service(world: World) {
  return (cmd: RpcCommand): unknown => {
    const name = String(cmd.name ?? '');
    if (name !== 'long-horizon.graph') throw Object.assign(new Error(`Unknown service: ${name}`), { code: 'E_NOT_FOUND' });
    const p = (cmd.payload ?? {}) as Record<string, unknown>;
    const find = (id?: unknown) => (id ? [world.current, ...world.past].find((g) => g?.graphId === id) : world.current ?? world.past[0]);
    if (p.op === 'snapshot') {
      const g = find(p.graphId);
      if (p.graphId && !g) return { ok: false, code: 'lh.graph.not_found', reason: 'unknown graph' };
      return { ok: true, graph: g ?? null };
    }
    if (p.op === 'history') return { ok: true, graphs: [world.current, ...world.past].filter(Boolean).map((g) => headOf(g!)) };
    if (p.op === 'retry' && p.target === 'goal') {
      if (!world.goal?.errored) return { ok: false, code: 'lh.goal.not_errored', reason: 'not errored' };
      world.goal.errored = false;
      return { ok: true };
    }
    if (p.op === 'retry') {
      const g = find(p.graphId);
      if (!g) return { ok: false, code: 'lh.graph.not_found', reason: 'x' };
      if (g.state !== 'open') return { ok: false, code: 'lh.graph.closed', reason: 'closed' };
      const w = g.work.find((x) => x.workId === p.workId);
      if (!w) return { ok: false, code: 'lh.work.not_found', reason: 'x' };
      if (w.status !== 'errored') return { ok: false, code: 'lh.work.not_continuable', reason: 'Already retrying.' };
      const ts = Date.now();
      w.status = 'running';
      w.canRetry = false;
      delete w.error;
      w.continues = (w.continues as number) + 1;
      w.events.push({ ts, status: 'continued', by: 'user', detail: 'continued by user' }, { ts: ts + 1, status: 'running', by: 'user', detail: `resumed operator ${String(w.operatorId)}` });
      g.counts = countsOf(g.work);
      g.updatedAt = ts;
      return { ok: true, workId: w.workId };
    }
    return { ok: false, code: 'lh.request.invalid', reason: 'unknown op' };
  };
}

function scenario(name: string, mode: 'graph' | 'swarm' | 'goal' | 'ralph', world: World, extraTopics: unknown[] = []): MockScenario {
  const cur = world.current;
  const progress =
    mode === 'goal'
      ? { mode: 'goal', goal: { objective: 'Make the long-horizon views readable on a phone', status: world.goal?.errored ? 'errored' : 'active', turn: 4, maxTurns: 12, ...(world.goal?.errored ? { error: 'transient (5xx): 503 Service Unavailable' } : {}) } }
      : mode === 'ralph'
        ? { mode: 'ralph', ralph: { name: 'cleanup-sweep', iteration: 3, checked: 5, total: 9 }, todos: '5/9 · ▶ Port the settings pages' }
        : { mode, graph: { graphId: cur?.graphId, mode, task: cur?.task, counts: cur?.counts } };
  return {
    name,
    hosts: [
      host([topic('unipi:long-horizon:state', { mode, owner: { kind: mode, status: 'active' } }), topic('unipi:long-horizon:progress', progress), ...extraTopics], {
        '_oino/extensions': () => ({ extensions: LH_EXTENSIONS }),
        '_oino/agents': () => ({ agents: [] }),
        '_oino/service_call': (cmd) => service(world)(cmd),
        prompt: () => ({}),
      }),
    ],
    setup: seedSessionRoute,
  };
}

export const lhGraphBig: MockScenario = scenario('lh-graph-big', 'graph', { current: bigGraph(), past: history() });
export const lhHistory: MockScenario = scenario('lh-history', 'graph', { current: smallGraph(), past: history() });
export const lhSwarm: MockScenario = scenario('lh-swarm', 'swarm', {
  current: graph({
    id: 'graph-sw-1',
    mode: 'swarm',
    task: 'Audit every settings page for phone overflow',
    state: 'open',
    createdAt: NOW - 14 * MIN,
    specs: ['completed', 'completed', 'running', 'running', 'errored', 'completed', 'queued', 'queued'].map((status, i) => ({ id: `page-${i + 1}`, status, i, start: NOW - (14 - i) * MIN })),
  }),
  past: history().slice(0, 1),
});
export const lhGoal: MockScenario = scenario('lh-goal', 'goal', { current: undefined, past: [], goal: { errored: false } });
export const lhGoalErrored: MockScenario = scenario('lh-goal-errored', 'goal', { current: undefined, past: [], goal: { errored: true } });
export const lhRalph: MockScenario = scenario('lh-ralph', 'ralph', { current: undefined, past: [] });
/** The old `status-lh-graph` scenario's seven nodes through the service. */
export const lhStatusWorld = (): World => ({ current: smallGraph(), past: history() });
export { service as lhService };
