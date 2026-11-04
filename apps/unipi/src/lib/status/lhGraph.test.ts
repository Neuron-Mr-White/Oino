import { describe, expect, it } from 'vitest';
import {
  LH_CODES,
  countWork,
  eventTitle,
  graphTimeline,
  layoutGraph,
  lhCodeText,
  newestFirst,
  parseHistory,
  parseSnapshot,
  progressSegments,
  refusalText,
  visibleWork,
  workElapsedMs,
  type LhWork,
} from './lhGraph';
import { lhService, lhGraphBig } from '../../dev/mock/scenarios/lh';

const w = (id: string, status: string, extra: Partial<LhWork> = {}, events: LhWork['events'] = []): LhWork =>
  ({ workId: id, status, instruction: '', inputIds: [], attempts: 0, autoContinues: 0, continues: 0, toolCalls: 0, canRetry: status === 'errored', hasOutput: false, events, eventsDropped: 0, ...extra }) as LhWork;

describe('parse', () => {
  it('snapshot: data → typed graph; null / refusal → undefined', () => {
    expect(parseSnapshot({ ok: true, graph: null })).toBeUndefined();
    expect(parseSnapshot({ ok: false, code: 'lh.graph.not_found' })).toBeUndefined();
    expect(parseSnapshot('junk')).toBeUndefined();
    const g = parseSnapshot({
      ok: true,
      graph: { graphId: 'g1', mode: 'swarm', task: 'T', state: 'finished', createdAt: 1, updatedAt: 5, counts: { completed: 2, errored: 1 }, finish: { reason: 'done', at: 5 }, events: [{ ts: 1, kind: 'opened', by: 'lead' }, { bad: 1 }], work: [{ workId: 'a', status: 'errored', model: 'zai/glm-5.3', events: [{ ts: 2, status: 'errored', by: 'host', detail: 'x' }] }, { nope: 1 }] },
    })!;
    expect(g.mode).toBe('swarm');
    expect(g.counts.errored).toBe(1);
    expect(g.counts.running).toBe(0);
    expect(g.finishReason).toBe('done');
    expect(g.events).toHaveLength(1);
    expect(g.work).toHaveLength(1);
    expect(g.work[0]!.canRetry).toBe(true);
  });
  it('history rows', () => {
    expect(parseHistory({ ok: true, graphs: [{ graphId: 'a', task: 'A', counts: {} }, {}] })).toHaveLength(1);
    expect(parseHistory(undefined)).toEqual([]);
  });
  it('the mock service answers the documented ops', () => {
    const big = lhGraphBig.hosts![0]!;
    expect(big).toBeTruthy();
    const svc = lhService({ current: undefined, past: [] });
    expect(parseSnapshot(svc({ type: '_oino/service_call', name: 'long-horizon.graph', payload: { op: 'snapshot' } }))).toBeUndefined();
  });
});

describe('error codes', () => {
  it('every code has text (exhaustive switch) and refusals map', () => {
    for (const c of LH_CODES) expect(lhCodeText(c).length).toBeGreaterThan(3);
    expect(refusalText({ ok: false, code: 'lh.work.continue_limit' })).toContain('Retried too often');
    expect(refusalText({ ok: false, code: 'lh.future.thing', reason: 'because' })).toBe('because');
    expect(refusalText({ ok: true })).toBeUndefined();
  });
});

describe('view shaping', () => {
  const list = [w('a', 'completed', {}, [{ ts: 10, status: 'completed', by: 'host' }]), w('b', 'running', {}, [{ ts: 30, status: 'running', by: 'host' }]), w('c', 'errored', {}, [{ ts: 20, status: 'errored', by: 'host' }]), w('d', 'superseded'), w('e', 'failed')];
  it('newest first by last event; no events keep reverse creation order', () => {
    expect(newestFirst(list).map((x) => x.workId)).toEqual(['b', 'c', 'a', 'e', 'd']);
  });
  it('Show errors off hides errored / failed / superseded', () => {
    expect(visibleWork(list, false).map((x) => x.workId)).toEqual(['a', 'b']);
    expect(visibleWork(list, true)).toHaveLength(5);
  });
  it('segments put errored apart from failed', () => {
    const segs = progressSegments(countWork(list));
    expect(segs.map((s) => s.status)).toEqual(['completed', 'running', 'errored', 'failed']);
    expect(Math.round(segs.reduce((n, s) => n + s.pct, 0))).toBe(80);
  });
  it('elapsed sums running spans; live to now', () => {
    expect(workElapsedMs({ events: [{ ts: 0, status: 'queued', by: 'lead' }] }, 100)).toBeUndefined();
    expect(workElapsedMs({ events: [{ ts: 10, status: 'running', by: 'host' }, { ts: 40, status: 'errored', by: 'host' }, { ts: 50, status: 'running', by: 'user' }] }, 90)).toBe(70);
  });
  it('timeline merges graph + node events, newest first, honours Show errors, says who continued', () => {
    const t = graphTimeline({ events: [{ ts: 1, kind: 'opened', by: 'lead' }, { ts: 99, kind: 'finished', by: 'lead' }], work: [w('c', 'errored', {}, [{ ts: 20, status: 'errored', by: 'host' }, { ts: 25, status: 'continued', by: 'user' }]), w('a', 'completed', {}, [{ ts: 10, status: 'completed', by: 'host' }])] });
    expect(t.map((e) => e.kind)).toEqual(['finished', 'continued', 'errored', 'completed', 'opened']);
    expect(t[1]!.title).toBe('Continued by user');
    expect(t[1]!.workId).toBe('c');
    expect(graphTimeline({ events: [], work: [w('c', 'errored', {}, [{ ts: 1, status: 'errored', by: 'host' }])] }, false)).toHaveLength(0);
    expect(eventTitle('continued', 'auto')).toBe('Continued by auto');
  });
});

describe('layout', () => {
  it('layers by inputs, wraps wide layers, 40 nodes stay bounded', () => {
    const nodes = [w('a', 'completed'), w('b', 'queued', { inputIds: ['r-a'] }), w('c', 'queued', { inputIds: ['r-b'], supersededBy: 'a' })];
    const l = layoutGraph(nodes);
    expect(l.nodes.map((n) => n.layer)).toEqual([0, 1, 2]);
    expect(l.edges).toHaveLength(3);
    const wide = layoutGraph(Array.from({ length: 40 }, (_, i) => w(`s${i}`, 'queued')), 3);
    expect(wide.nodes).toHaveLength(40);
    expect(wide.width).toBeLessThan(700);
    expect(new Set(wide.nodes.map((n) => `${n.x},${n.y}`)).size).toBe(40);
  });
  it('survives a cycle', () => {
    expect(layoutGraph([w('a', 'queued', { inputIds: ['r-b'] }), w('b', 'queued', { inputIds: ['r-a'] })]).nodes).toHaveLength(2);
  });
});
