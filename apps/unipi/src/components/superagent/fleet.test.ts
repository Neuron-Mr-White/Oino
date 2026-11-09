import { describe, expect, it } from 'vitest';
import type { FleetNode, SuperagentSession } from './api';
import { StatusSchema } from './api';
import { buildTree, CARD_H, CARD_W, clampScale, fallbackFleet, fitView, filterTree, fleetCounts, layoutFleet, MAX_SCALE, MIN_SCALE, openTarget, RESET_VIEW, UNGROUPED, zoomAt, type Rect } from './fleet';

const node = (over: Partial<FleetNode> & { id: string }): FleetNode => ({ parentId: null, project: 'Oino', projectId: 'oino', title: over.id, mode: 'plain', role: 'subagent', status: 'working', model: null, startedAt: null, endedAt: null, ...over });
const lead = (over: Partial<FleetNode> & { id: string }) => node({ role: 'lead', ...over });
const apart = (a: Rect, b: Rect) => a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
const pairwiseDisjoint = (rects: Rect[]) => rects.every((a, i) => rects.slice(i + 1).every((b) => apart(a, b)));

describe('fallback fleet', () => {
  const status = StatusSchema.parse({
    enabled: true,
    leads: [
      { projectId: 'oino', sessionId: 'lead-oino', autonomy: 'high', mode: 'graph', model: 'openai/gpt-5', startedAt: '2026-11-09T10:00:00.000Z', ticketIds: [] },
      { projectId: 'web', sessionId: 'lead-web', autonomy: 'high', mode: 'swarm', ticketIds: [] },
      { projectId: 'hp', sessionId: 'lead-hp', autonomy: 'high', mode: 'goal', ticketIds: [] },
    ],
    needsYou: [{ projectId: 'web', taskId: 'WEB-8', status: 'blocked', workerSessionId: 'lead-web', requestId: 'r', sessionId: 'lead-web', tool: 'bash', command: 'x', summary: 'x', risk: 'high', ceiling: 'medium', reason: 'x' }],
    projects: [{ projectId: 'oino', name: 'Oino', autoStart: true }, { projectId: 'web', name: 'Website', autoStart: true }],
  });
  const row = (sessionId: string, over: Partial<SuperagentSession> = {}): SuperagentSession => ({ sessionId, title: sessionId, createdAt: 1000, updatedAt: 2000, path: `/${sessionId}`, ...over });

  it('derives leads, running rows and ended rows, without subagents', () => {
    const fleet = fallbackFleet(status, [row('lead-oino', { state: { kind: 'working' } }), row('chat-1', { state: { kind: 'working' }, project: 'Oino' }), row('old-1', { project: 'Website', updatedAt: 5000 })]);
    const by = Object.fromEntries(fleet.nodes.map((n) => [n.id, n]));
    expect(by['lead-oino']).toMatchObject({ role: 'lead', mode: 'graph', status: 'working', project: 'Oino', projectId: 'oino', model: 'openai/gpt-5', title: 'Lead · Oino' });
    expect(by['lead-oino']!.startedAt).toBe(Date.parse('2026-11-09T10:00:00.000Z'));
    expect(by['lead-web']).toMatchObject({ mode: 'swarm', status: 'waiting', project: 'Website' });
    expect(by['lead-hp']).toMatchObject({ mode: 'plain', status: 'working', project: 'hp' });
    expect(by['chat-1']).toMatchObject({ status: 'working', projectId: 'oino' });
    expect(by['old-1']).toMatchObject({ status: 'ended', endedAt: 5000, projectId: 'web' });
    expect(fleet.nodes.filter((n) => n.role === 'subagent')).toHaveLength(0);
    expect(fleet.nodes.filter((n) => n.id === 'lead-oino')).toHaveLength(1); // the running history row is the lead, not a second node
  });

  it('is empty when the host knows nothing, and keeps only the newest finished rows', () => {
    expect(fallbackFleet(undefined, undefined).nodes).toEqual([]);
    const many = Array.from({ length: 60 }, (_, i) => row(`h${i}`, { updatedAt: i }));
    const ended = fallbackFleet(undefined, many).nodes;
    expect(ended).toHaveLength(40);
    expect(ended[0]!.id).toBe('h59');
  });
});

describe('dedupe, parents and regions', () => {
  it('dedupes by id: the later report wins', () => {
    const fleet = [node({ id: 'a', status: 'idle' }), node({ id: 'b' }), node({ id: 'a', status: 'working' })];
    expect(fleetCounts(fleet)).toEqual({ working: 2, waiting: 0, idle: 0, ended: 0, all: 2 });
    expect([...buildTree({ nodes: fleet }).nodes.keys()]).toEqual(['a', 'b']);
  });

  it('hangs a missing-parent node on its project lead, else on Ungrouped (rendered last)', () => {
    const tree = buildTree({ nodes: [
      node({ id: 'stray-1', parentId: 'gone', project: 'Website', projectId: 'web' }), // no Website lead: Ungrouped
      lead({ id: 'lead-oino' }),
      node({ id: 'orphan-oino', parentId: 'gone' }), // Oino has a lead: attaches
      node({ id: 'nowhere', project: null, projectId: null }),
      node({ id: 'child', parentId: 'lead-oino' }),
    ] });
    expect(tree.parent.get('orphan-oino')).toBe('lead-oino');
    expect(tree.parent.get('stray-1')).toBeNull();
    expect(tree.group.get('stray-1')).toBe(UNGROUPED);
    expect(tree.group.get('nowhere')).toBe(UNGROUPED);
    expect(tree.group.get('orphan-oino')).toBe('oino');
    expect(tree.groups.map((g) => g.key)).toEqual(['oino', UNGROUPED]);
    expect(tree.groups.at(-1)!.ungrouped).toBe(true);
    expect(tree.groups.at(-1)!.roots).toEqual(['stray-1', 'nowhere']);
  });

  it('keeps the lead first among roots and survives a parent cycle', () => {
    const tree = buildTree({ nodes: [node({ id: 'solo' }), lead({ id: 'L' }), node({ id: 'x', parentId: 'y' }), node({ id: 'y', parentId: 'x' })] });
    expect(tree.groups[0]!.roots[0]).toBe('L');
    const roots = tree.groups[0]!.roots;
    expect(roots).toContain('solo');
    expect(roots.filter((r) => r === 'x' || r === 'y')).toHaveLength(1);
  });

  it('uses the edge list when a node carries no parentId', () => {
    const tree = buildTree({ nodes: [lead({ id: 'L' }), node({ id: 'c' })], edges: [{ from: 'L', to: 'c' }] });
    expect(tree.parent.get('c')).toBe('L');
  });
});

describe('counts and filters', () => {
  const nodes = [
    lead({ id: 'L1' }), node({ id: 's1', parentId: 'L1', status: 'idle' }), node({ id: 's2', parentId: 's1', status: 'working' }), node({ id: 's3', parentId: 'L1', status: 'ended' }),
    lead({ id: 'L2', project: 'Website', projectId: 'web', status: 'idle' }), node({ id: 'w1', parentId: 'L2', project: 'Website', projectId: 'web', status: 'waiting' }),
  ];
  it('counts per status and in total', () => {
    expect(fleetCounts(nodes)).toEqual({ working: 2, waiting: 1, idle: 2, ended: 1, all: 6 });
  });
  it('keeps matches plus their ancestors, dimmed', () => {
    const tree = buildTree({ nodes });
    const f = filterTree(tree, 'working');
    expect([...f.visible].sort()).toEqual(['L1', 's1', 's2']);
    expect([...f.dimmed].sort()).toEqual(['s1']); // L1 works itself, s1 only connects
    expect(f.matches).toBe(2);
    const waiting = filterTree(tree, 'waiting');
    expect([...waiting.visible].sort()).toEqual(['L2', 'w1']);
    expect([...waiting.dimmed]).toEqual(['L2']);
    expect(filterTree(tree, 'all').dimmed.size).toBe(0);
  });
  it('hides regions without a match', () => {
    const tree = buildTree({ nodes });
    const working = layoutFleet(tree, filterTree(tree, 'working'));
    expect(working.regions.map((r) => r.key)).toEqual(['oino']);
    expect(working.nodes.map((n) => n.id).sort()).toEqual(['L1', 's1', 's2']);
    expect(layoutFleet(tree, filterTree(tree, 'ended')).regions.map((r) => r.key)).toEqual(['oino']);
    expect(layoutFleet(tree, filterTree(tree, 'waiting')).regions.map((r) => r.key)).toEqual(['web']);
    expect(layoutFleet(buildTree({ nodes: [] })).nodes).toEqual([]);
  });
});

describe('layout', () => {
  const wide = [
    lead({ id: 'L1', title: 'A project with a rather long name indeed' }),
    ...['a', 'b', 'c', 'd'].map((id) => node({ id, parentId: 'L1' })),
    node({ id: 'a1', parentId: 'a' }), node({ id: 'a2', parentId: 'a' }), node({ id: 'a3', parentId: 'a' }),
    lead({ id: 'L2', project: 'Website', projectId: 'web' }), node({ id: 'w', parentId: 'L2', project: 'Website', projectId: 'web' }),
    lead({ id: 'L3', project: 'Heartpick', projectId: 'hp' }),
    lead({ id: 'L4', project: 'Orchard', projectId: 'orch' }), lead({ id: 'L5', project: 'Billing', projectId: 'bill' }),
    node({ id: 'free', project: null, projectId: null }),
  ];
  const layout = layoutFleet(buildTree({ nodes: wide }), undefined, 700);

  it('never overlaps nodes or regions, at fixed card size', () => {
    expect(layout.nodes).toHaveLength(wide.length);
    expect(layout.nodes.every((n) => n.w === CARD_W && n.h === CARD_H)).toBe(true);
    expect(pairwiseDisjoint(layout.nodes)).toBe(true);
    expect(pairwiseDisjoint(layout.regions)).toBe(true);
    expect(layout.regions.at(-1)!.ungrouped).toBe(true);
  });
  it('wraps regions into rows and keeps every node inside its region', () => {
    expect(new Set(layout.regions.map((r) => r.y)).size).toBeGreaterThan(1);
    for (const n of layout.nodes) {
      const r = layout.regions.find((x) => x.key === n.region)!;
      expect(n.x).toBeGreaterThanOrEqual(r.x); expect(n.y).toBeGreaterThanOrEqual(r.y);
      expect(n.x + n.w).toBeLessThanOrEqual(r.x + r.w); expect(n.y + n.h).toBeLessThanOrEqual(r.y + r.h);
    }
    for (const r of layout.regions) { expect(r.x + r.w).toBeLessThanOrEqual(layout.width); expect(r.y + r.h).toBeLessThanOrEqual(layout.height); }
  });
  it('puts each depth on one row and centres a parent over its children', () => {
    const at = (id: string) => layout.nodes.find((n) => n.id === id)!;
    expect(new Set(['a', 'b', 'c', 'd'].map((id) => at(id).y)).size).toBe(1);
    expect(at('a').y).toBeGreaterThan(at('L1').y);
    expect(at('a1').y).toBeGreaterThan(at('a').y);
    const kids = ['a', 'b', 'c', 'd'].map(at);
    expect(at('L1').x + CARD_W / 2).toBeCloseTo((kids[0]!.x + kids[3]!.x + CARD_W) / 2);
    const under = ['a1', 'a2', 'a3'].map(at);
    expect(at('a').x + CARD_W / 2).toBeCloseTo((under[0]!.x + under[2]!.x + CARD_W) / 2);
  });
  it('draws one edge per parent link, from the parent bottom to the child top', () => {
    expect(layout.edges).toHaveLength(wide.filter((n) => n.parentId).length);
    const e = layout.edges.find((x) => x.to === 'a1')!;
    const a = layout.nodes.find((n) => n.id === 'a')!; const a1 = layout.nodes.find((n) => n.id === 'a1')!;
    expect(e.from).toBe('a');
    expect(e.d.startsWith(`M${a.x + CARD_W / 2} ${a.y + CARD_H}`)).toBe(true);
    expect(e.d.endsWith(`V${a1.y}`)).toBe(true);
  });
  it('a lone lead is a single-node region', () => {
    const one = layoutFleet(buildTree({ nodes: [lead({ id: 'only', project: 'Heartpick', projectId: 'hp' })] }));
    expect(one.regions).toHaveLength(1);
    expect(one.nodes).toHaveLength(1);
    expect(one.edges).toHaveLength(0);
  });
});

describe('view maths', () => {
  it('clamps the scale to 0.25 to 2', () => {
    expect(clampScale(0.01)).toBe(MIN_SCALE);
    expect(clampScale(9)).toBe(MAX_SCALE);
  });
  it('fits large content into the viewport with padding, centred', () => {
    const v = fitView({ width: 2000, height: 1000 }, { width: 1000, height: 800 }, 40);
    expect(v.scale).toBeCloseTo(920 / 2000);
    expect(v.x).toBeCloseTo((1000 - 2000 * v.scale) / 2);
    expect(v.y).toBeCloseTo((800 - 1000 * v.scale) / 2);
    expect(2000 * v.scale + 2 * 40).toBeLessThanOrEqual(1000 + 1e-6);
  });
  it('never magnifies small content past 1 and clamps tiny scales', () => {
    expect(fitView({ width: 200, height: 100 }, { width: 1000, height: 800 }).scale).toBe(1);
    expect(fitView({ width: 100000, height: 100 }, { width: 1000, height: 800 }).scale).toBe(MIN_SCALE);
    expect(fitView({ width: 0, height: 0 }, { width: 1000, height: 800 })).toEqual(RESET_VIEW);
    expect(fitView({ width: 100, height: 100 }, { width: 0, height: 0 })).toEqual(RESET_VIEW);
  });
  it('zooms around a point: that point stays put, within the clamp', () => {
    const v = zoomAt({ x: 10, y: 20, scale: 1 }, 2, 300, 200);
    expect(v.scale).toBe(2);
    expect((300 - v.x) / v.scale).toBeCloseTo((300 - 10) / 1);
    expect((200 - v.y) / v.scale).toBeCloseTo((200 - 20) / 1);
    expect(zoomAt({ x: 0, y: 0, scale: 1.9 }, 5, 0, 0).scale).toBe(MAX_SCALE);
    expect(zoomAt({ x: 0, y: 0, scale: 0.3 }, 0.1, 0, 0).scale).toBe(MIN_SCALE);
  });
});

describe('opening a node', () => {
  it('a lead or worker opens itself, a subagent the nearest lead or worker above it', () => {
    const tree = buildTree({ nodes: [lead({ id: 'L' }), node({ id: 'w', parentId: 'L', role: 'worker' }), node({ id: 's', parentId: 'w' }), node({ id: 'deep', parentId: 's' }), node({ id: 'lost', parentId: 'gone', project: null, projectId: null })] });
    expect(openTarget(tree, 'L')).toBe('L');
    expect(openTarget(tree, 'w')).toBe('w');
    expect(openTarget(tree, 'deep')).toBe('w');
    expect(openTarget(tree, 'lost')).toBe('lost');
  });
});
