import { describe, expect, it } from 'vitest';
import type { FleetNode, SuperagentSession } from './api';
import { FleetSchema, StatusSchema } from './api';
import {
  age, BAND, buildTree, clock, DEFAULT_FILTER, detailLine, fallbackFleet, filterTree, fitView, fleetCounts, GUTTER, intersects, layoutFleet, lodOf, makeFleet, makeNode, mergeStatusLeads, navOf, nextNeedingYou,
  nodeTitle, openTarget, panToReveal, RESET_VIEW, searchMatches, stateText, worldRect, zoomAt, MAX_SCALE, MIN_SCALE, type CanvasFilter, type Layout, type Rect,
} from './fleet';

const node = (over: Partial<FleetNode> & { id: string }): FleetNode => makeNode({ project: 'Oino', projectId: 'oino', groupKey: 'oino', role: 'subagent', status: 'working', ...over });
const lead = (over: Partial<FleetNode> & { id: string }) => node({ role: 'lead', ...over });
const SA = node({ id: 'sa', role: 'superagent', project: null, projectId: null, groupKey: null, title: 'Router' });
const GROUPS = [{ key: 'oino', name: 'Oino', hasBoard: true }, { key: 'web', name: 'Website', hasBoard: true }, { key: 'other', name: 'Other chats', hasBoard: false }];
const fleetOf = (nodes: FleetNode[], over: Partial<Parameters<typeof makeFleet>[0]> = {}) => makeFleet({ nodes, groups: GROUPS, superagentId: nodes.some((n) => n.id === 'sa') ? 'sa' : null, rev: 'r1', ...over });
const F: CanvasFilter = { ...DEFAULT_FILTER };
const lay = (nodes: FleetNode[], opts: { filter?: CanvasFilter; density?: 'compact' | 'detailed'; expanded?: Set<string>; wrap?: number; collapsed?: Set<string> } = {}) => {
  const tree = buildTree(fleetOf(nodes));
  return { tree, layout: layoutFleet(tree, opts.filter ?? F, opts.density ?? 'detailed', opts.expanded ?? new Set(), opts.wrap ?? 2000, opts.collapsed) };
};
const apart = (a: Rect, b: Rect) => a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
const pairwiseDisjoint = (rects: Rect[]) => rects.every((a, i) => rects.slice(i + 1).every((b) => apart(a, b)));
const within = (inner: Rect, outer: Rect) => inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h;
/** Corner points of an edge path: the arcs' control points are the polyline's corners. */
const points = (d: string): [number, number][] => [...d.matchAll(/(-?\d+(?:\.\d+)?) (-?\d+(?:\.\d+)?)/g)].map((m) => [Number(m[1]), Number(m[2])]);
const nodeRect = (l: Layout, id: string) => l.nodes.find((n) => n.id === id)!;

const project = (key: string, n: number, over: Partial<FleetNode> = {}): FleetNode[] => [
  lead({ id: `lead-${key}`, groupKey: key, projectId: key, project: key, startedAt: 1 }),
  ...Array.from({ length: n }, (_, i) => node({ id: `${key}-${i}`, parentId: `lead-${key}`, groupKey: key, projectId: key, project: key, startedAt: 10 + i, ...over })),
];

describe('schema', () => {
  it('parses today\'s older payload: role lead for everything, no groups, no rev', () => {
    const f = FleetSchema.parse({ nodes: [{ id: 'a', parentId: null, project: null, projectId: null, title: 'x', mode: 'plain', role: 'lead', status: 'working', model: null, startedAt: 1, endedAt: null }], edges: [] });
    expect(f).toMatchObject({ superagentId: null, groups: [], rev: '' });
    expect(f.nodes[0]).toMatchObject({ groupKey: null, name: null, cwd: null, ticketIds: [], step: null, tool: null, question: null, health: null, lastActiveAt: null, idleReason: null });
  });
  it('parses every new field and falls back on unknown enums', () => {
    const f = FleetSchema.parse({ superagentId: 's', rev: 'abc', groups: [{ key: 'oino', name: 'Oino', hasBoard: true }], nodes: [
      { id: 's', role: 'superagent', status: 'stalled', health: { error: 'x', attempts: 2, since: 5 }, question: { taskId: 'T-1', excerpt: 'q' }, idleReason: 'unloaded', ticketIds: ['T-1'] },
      { id: 'b', role: 'martian', status: 'asleep' },
    ] });
    expect(f.nodes[0]).toMatchObject({ role: 'superagent', status: 'stalled', health: { error: 'x', attempts: 2, since: 5 }, question: { taskId: 'T-1', excerpt: 'q' }, idleReason: 'unloaded' });
    expect(f.nodes[1]).toMatchObject({ role: 'subagent', status: 'idle' });
  });
});

describe('fallback fleet', () => {
  const status = StatusSchema.parse({
    enabled: true,
    leads: [
      { projectId: 'oino', sessionId: 'lead-oino', autonomy: 'high', mode: 'graph', model: 'openai/gpt-5', startedAt: '2026-11-09T10:00:00.000Z', ticketIds: ['OIN-1'], step: 'Testing', tool: 'bash' },
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
    expect(by['lead-oino']).toMatchObject({ role: 'lead', mode: 'graph', status: 'working', project: 'Oino', projectId: 'oino', model: 'openai/gpt-5', ticketIds: ['OIN-1'], step: 'Testing', tool: 'bash' });
    expect(by['lead-oino']!.startedAt).toBe(Date.parse('2026-11-09T10:00:00.000Z'));
    expect(by['lead-web']).toMatchObject({ mode: 'swarm', status: 'waiting', project: 'Website' });
    expect(by['lead-hp']).toMatchObject({ mode: 'plain', status: 'working', project: 'hp' });
    expect(by['chat-1']).toMatchObject({ role: 'chat', status: 'working', projectId: 'oino' });
    expect(by['old-1']).toMatchObject({ role: 'chat', status: 'ended', endedAt: 5000, projectId: 'web' });
    expect(fleet.nodes.filter((n) => n.role === 'subagent')).toHaveLength(0);
    expect(fleet.nodes.filter((n) => n.id === 'lead-oino')).toHaveLength(1);
  });

  it('overlays live status leads onto older fleet snapshots', () => {
    const fleet = mergeStatusLeads(makeFleet({ nodes: [lead({ id: 'lead-oino', project: null, projectId: null, status: 'idle' })] }), status);
    const tree = buildTree(fleet);
    expect(tree.nodes.get('lead-oino')).toMatchObject({ role: 'lead', status: 'working', projectId: 'oino', project: 'Oino' });
    expect(tree.group.get('lead-oino')).toBe('oino');
    expect(filterTree(tree, { ...F, statuses: ['working'] }).visible.has('lead-web')).toBe(false); // awaiting approval
  });

  it('is empty when the host knows nothing, and keeps only the newest finished rows', () => {
    expect(fallbackFleet(undefined, undefined).nodes).toEqual([]);
    const ended = fallbackFleet(undefined, Array.from({ length: 60 }, (_, i) => row(`h${i}`, { updatedAt: i }))).nodes;
    expect(ended).toHaveLength(40);
    expect(ended[0]!.id).toBe('h59');
  });
});

describe('words', () => {
  it('titles are Superagent, Lead, the spec name or the chat title, never "Agent"', () => {
    expect(nodeTitle(node({ id: 'a', role: 'superagent', title: 'Router' }))).toBe('Superagent');
    expect(nodeTitle(node({ id: 'a', role: 'lead', title: 'Lead · Oino' }))).toBe('Lead');
    expect(nodeTitle(node({ id: 'a', name: 'explore', title: 'Scan the repo' }))).toBe('explore');
    expect(nodeTitle(node({ id: 'a', title: 'Agent' }))).toBe('Subagent');
    expect(nodeTitle(node({ id: 'a', role: 'worker', title: '' }))).toBe('Worker');
    expect(nodeTitle(node({ id: 'a', role: 'chat', title: 'Fix the flaky test' }))).toBe('Fix the flaky test');
    expect(nodeTitle(node({ id: 'a', role: 'chat', title: 'Untitled' }))).toBe('Chat');
  });
  it('state text and detail line per state', () => {
    const now = 10 * 60_000;
    expect(stateText(node({ id: 'a', status: 'working', startedAt: now - 4 * 60_000 }), now)).toBe('Working · 4m');
    expect(stateText(node({ id: 'a', status: 'waiting' }), now)).toBe('Needs you');
    expect(stateText(node({ id: 'a', status: 'stalled' }), now)).toBe('Stalled');
    expect(stateText(node({ id: 'a', status: 'idle' }), now)).toBe('Idle');
    expect(stateText(node({ id: 'a', status: 'ended', endedAt: 1000 }), 2 * 3_600_000 + 1000)).toBe('Done 2h ago');
    expect(detailLine(node({ id: 'a', status: 'waiting', question: { taskId: 'T', excerpt: 'Which db?' } }))).toBe('Which db?');
    expect(detailLine(node({ id: 'a', status: 'stalled', health: { error: '429', attempts: 3, since: 1 } }))).toBe('429');
    expect(detailLine(node({ id: 'a', status: 'working', step: 'Testing', tool: 'bash' }))).toBe('Testing · bash');
    expect(age(65_000)).toBe('1m'); expect(age(3 * 3_600_000 + 5 * 60_000)).toBe('3h 05m'); expect(clock(42_000)).toBe('0:42');
  });
});

describe('tree and groups', () => {
  it('keeps the host\'s group order, children by startedAt then id, subagents in their root\'s group', () => {
    const tree = buildTree(fleetOf([SA, ...project('web', 1), ...project('oino', 0), node({ id: 'z', parentId: 'lead-oino', startedAt: 5 }), node({ id: 'y', parentId: 'lead-oino', startedAt: 5 }), node({ id: 'x', parentId: 'lead-oino', startedAt: 2 })]));
    expect(tree.groups.map((g) => g.key)).toEqual(['oino', 'web']);
    expect(tree.children.get('lead-oino')).toEqual(['x', 'y', 'z']);
    expect(tree.group.get('x')).toBe('oino');
    expect(tree.group.get('sa')).toBe(BAND);
  });
  it('puts router chats in the band and project chats in their group; unknown folders go to Other chats', () => {
    const tree = buildTree(fleetOf([SA, node({ id: 'rc1', role: 'chat', groupKey: null, projectId: null, project: null, startedAt: 1 }), node({ id: 'rc2', role: 'chat', groupKey: null, projectId: null, project: null, startedAt: 2 }),
      node({ id: 'c1', role: 'chat', groupKey: 'oino' }), node({ id: 'c2', role: 'chat', groupKey: 'other', projectId: null, project: null }), node({ id: 'cs', parentId: 'c1', groupKey: 'oino' })]));
    expect(tree.earlier).toEqual(['rc2', 'rc1']);
    expect(tree.group.get('c1')).toBe('oino'); expect(tree.group.get('cs')).toBe('oino'); expect(tree.group.get('c2')).toBe('other');
    expect(tree.groups.map((g) => g.key)).toEqual(['oino', 'other']);
  });
  it('derives groups for an older host (no groups, no rev): board projects A-Z, nodes without a project in Other chats, never "Ungrouped"', () => {
    const old = (id: string, projectId: string | null, project: string | null) => makeNode({ id, role: 'lead', status: 'working', projectId, project });
    const tree = buildTree({ nodes: [old('a', 'zeta', 'Zeta'), old('b', null, null), old('c', 'alpha', 'Alpha')] });
    expect(tree.modern).toBe(false);
    expect(tree.groups.map((g) => g.name)).toEqual(['Alpha', 'Zeta', 'Other chats']);
    expect(tree.hasEdges).toBe(false);
    expect(tree.superagentId).toBeNull();
  });
  it('dedupes by id, re-roots orphans and cuts cycles', () => {
    expect(fleetCounts([node({ id: 'a', status: 'idle' }), node({ id: 'b' }), node({ id: 'a', status: 'working' })])).toEqual({ working: 2, waiting: 0, stalled: 0, idle: 0, ended: 0, all: 2 });
    const tree = buildTree(fleetOf([node({ id: 'o', parentId: 'gone' }), node({ id: 'c1', parentId: 'c2' }), node({ id: 'c2', parentId: 'c1' })]));
    expect(tree.parent.get('o')).toBeNull();
    expect([tree.parent.get('c1'), tree.parent.get('c2')].filter(Boolean)).toHaveLength(1);
    expect(tree.nodes.size).toBe(3);
  });
  it('openTarget walks up past subagents and workers', () => {
    const tree = buildTree(fleetOf([lead({ id: 'l' }), node({ id: 's', parentId: 'l' }), node({ id: 'w', role: 'worker', parentId: 's' }), node({ id: 'stray' })]));
    expect(openTarget(tree, 'w')).toBe('l'); expect(openTarget(tree, 'l')).toBe('l'); expect(openTarget(tree, 'stray')).toBe('stray');
  });
});

describe('filter', () => {
  const nodes = [lead({ id: 'l', status: 'idle' }), node({ id: 'a', parentId: 'l', status: 'working' }), node({ id: 'b', parentId: 'a', status: 'ended' }), ...project('web', 1, { status: 'ended' }).map((n) => ({ ...n, status: 'ended' as const })), node({ id: 'chat', role: 'chat', groupKey: 'oino', status: 'working' })];
  it('keeps matches and dims the ancestors kept for them', () => {
    const tree = buildTree(fleetOf(nodes));
    const f = filterTree(tree, { ...F, statuses: ['ended'] });
    expect([...f.visible].sort()).toEqual(['a', 'b', 'l', 'lead-web', 'web-0']);
    expect([...f.dimmed].sort()).toEqual(['a', 'l']);
    expect(f.hiddenGroups).toBe(0);
  });
  it('chats off hides chat trees; hidden projects drop whole groups and are counted', () => {
    const tree = buildTree(fleetOf(nodes));
    expect(filterTree(tree, { ...F, chats: false }).visible.has('chat')).toBe(false);
    const f = filterTree(tree, { ...F, hidden: ['web'] });
    expect(f.visible.has('web-0')).toBe(false);
    expect(f.hiddenGroups).toBe(1);
    expect(filterTree(tree, { ...F, statuses: ['stalled'] }).hiddenGroups).toBe(2);
  });
  it('search matches title, name, project and ticket ids', () => {
    const tree = buildTree(fleetOf([lead({ id: 'l', ticketIds: ['OIN-77'] }), node({ id: 'a', parentId: 'l', name: 'explore', title: 'Scan' }), node({ id: 'w', groupKey: 'web', project: 'Website', projectId: 'web' })]));
    expect([...searchMatches(tree, 'oin-77')]).toEqual(['l']);
    expect([...searchMatches(tree, 'EXPLO')]).toEqual(['a']);
    expect([...searchMatches(tree, 'website')]).toEqual(['w']);
    expect(searchMatches(tree, '  ').size).toBe(0);
  });
});

describe('layoutFleet', () => {
  const full = [SA, ...project('oino', 7), ...project('web', 3), node({ id: 'chat-a', role: 'chat', groupKey: 'oino', startedAt: 99 }), node({ id: 'chat-b', role: 'chat', groupKey: 'oino', startedAt: 100 }), node({ id: 'other-1', role: 'chat', groupKey: 'other', projectId: null, project: null })];

  it('lays out a tidy tree: parents above children, nothing overlaps, everything sits inside its group', () => {
    const { layout } = lay(full);
    expect(pairwiseDisjoint(layout.nodes)).toBe(true);
    expect(pairwiseDisjoint(layout.groups)).toBe(true);
    for (const n of layout.nodes) if (n.group !== BAND) expect(within(n, layout.groups.find((g) => g.key === n.group)!)).toBe(true);
    const parent = nodeRect(layout, 'lead-oino'); const child = nodeRect(layout, 'oino-0');
    expect(child.y).toBeGreaterThanOrEqual(parent.y + parent.h + 56); // level gap
    expect(layout.nodes.find((n) => n.id === 'sa')!.group).toBe(BAND);
  });

  it('wraps children into rows under the same bus and keeps the group inside its max width', () => {
    const { layout } = lay([...project('oino', 13)]);
    const ys = new Set(layout.nodes.filter((n) => n.parent === 'lead-oino').map((n) => n.y));
    expect(ys.size).toBeGreaterThan(1);
    expect(layout.groups[0]!.w).toBeLessThanOrEqual(1040);
    const first = [...ys].sort((a, b) => a - b)[0]!;
    expect(layout.nodes.filter((n) => n.parent === 'lead-oino' && n.y === first).length).toBeLessThanOrEqual(6);
    // every child still has its own connected edge that ends at its top centre
    for (const n of layout.nodes.filter((x) => x.parent === 'lead-oino')) {
      const e = layout.edges.find((x) => x.to === n.id && x.kind === 'base')!;
      const pts = points(e.d);
      expect(pts[pts.length - 1]).toEqual([n.x + n.w / 2, n.y]);
    }
  });

  it('compact uses more children per row and wider gaps', () => {
    const detailed = lay(project('oino', 7)).layout; const compact = lay(project('oino', 7), { density: 'compact' }).layout;
    expect(new Set(detailed.nodes.filter((n) => n.parent === 'lead-oino').map((n) => n.y)).size).toBeGreaterThan(1);
    expect(new Set(compact.nodes.filter((n) => n.parent === 'lead-oino').map((n) => n.y)).size).toBe(1);
    expect(nodeRect(compact, 'oino-0').w).toBe(128);
  });

  it('every parent-child edge leaves the parent bottom centre and enters the child top centre, in the same pass', () => {
    const { layout, tree } = lay(full);
    for (const n of layout.nodes) {
      if (!n.parent) continue;
      const p = nodeRect(layout, n.parent);
      for (const e of layout.edges.filter((x) => x.to === n.id && x.from === n.parent)) {
        const pts = points(e.d);
        expect(pts[0]).toEqual([p.x + p.w / 2, p.y + p.h]);
        expect(pts[pts.length - 1]).toEqual([n.x + n.w / 2, n.y]);
      }
    }
    expect(layout.edges.some((e) => e.from === 'lead-oino' && e.to === 'oino-0')).toBe(true);
    expect(tree.hasEdges).toBe(true);
  });

  it('shelf-packs groups in fixed order with 64px gutters, and a narrow wrap stacks one per row', () => {
    const wide = lay(full, { wrap: 3000 }).layout;
    expect(wide.groups.map((g) => g.key)).toEqual(['oino', 'web', 'other']);
    for (const a of wide.groups) for (const b of wide.groups) {
      if (a === b) continue;
      const hGap = Math.max(b.x - (a.x + a.w), a.x - (b.x + b.w)); const vGap = Math.max(b.y - (a.y + a.h), a.y - (b.y + b.h));
      expect(Math.max(hGap, vGap)).toBeGreaterThanOrEqual(GUTTER);
    }
    const narrow = lay(full, { wrap: 390 }).layout;
    expect(new Set(narrow.groups.map((g) => g.x)).size).toBe(1);
    expect(narrow.groups.map((g) => g.y)).toEqual([...narrow.groups.map((g) => g.y)].sort((a, b) => a - b));
  });

  it('draws the Superagent -> lead trunk through the gutters: no segment crosses a group, and it ends at the lead top', () => {
    for (const wrap of [1300, 2800, 390]) {
      const { layout } = lay(full, { wrap });
      const trunks = layout.edges.filter((e) => e.from === 'sa' && e.kind === 'base');
      expect(trunks.map((t) => t.to).sort()).toEqual(['lead-oino', 'lead-web']);
      for (const t of trunks) {
        const pts = points(t.d);
        const lead = nodeRect(layout, t.to);
        expect(pts[pts.length - 1]).toEqual([lead.x + lead.w / 2, lead.y]);
        // all but the last segment stay out of every group's interior
        for (let i = 0; i < pts.length - 2; i++) {
          const [a, b] = [pts[i]!, pts[i + 1]!];
          const seg: Rect = { x: Math.min(a[0], b[0]) - 0.5, y: Math.min(a[1], b[1]) - 0.5, w: Math.abs(a[0] - b[0]) + 1, h: Math.abs(a[1] - b[1]) + 1 };
          for (const g of layout.groups) expect(intersects(seg, g)).toBe(false);
        }
        // the final segment is a vertical drop into its own group
        const g = layout.groups.find((x) => x.key === lead.group)!;
        const [a, b] = [pts[pts.length - 2]!, pts[pts.length - 1]!];
        expect(a[0]).toBe(b[0]);
        for (const other of layout.groups.filter((x) => x !== g)) expect(intersects({ x: a[0] - 0.5, y: Math.min(a[1], b[1]), w: 1, h: Math.abs(a[1] - b[1]) }, other)).toBe(false);
      }
    }
  });

  it('centres the Superagent band over the first row and chats have no edge to it', () => {
    const { layout } = lay(full, { wrap: 3000 });
    const sa = nodeRect(layout, 'sa');
    const row1 = layout.groups.filter((g) => g.y === layout.groups[0]!.y);
    const from = Math.min(...row1.map((g) => g.x)); const to = Math.max(...row1.map((g) => g.x + g.w));
    expect(Math.abs(sa.x + sa.w / 2 - (from + to) / 2)).toBeLessThanOrEqual(1);
    expect(layout.edges.some((e) => e.from === 'sa' && ['chat-a', 'other-1'].includes(e.to))).toBe(false);
    expect(sa.y + sa.h).toBeLessThan(layout.groups[0]!.y);
  });

  it('a Superagent with subagents keeps the trunk beside them', () => {
    const { layout } = lay([SA, node({ id: 'sa-sub', parentId: 'sa', groupKey: null }), ...project('oino', 1)]);
    const sub = nodeRect(layout, 'sa-sub');
    const trunk = layout.edges.find((e) => e.from === 'sa' && e.to === 'lead-oino' && e.kind === 'base')!;
    for (const [x, y] of points(trunk.d).slice(0, -1)) expect(x > sub.x + sub.w || y < sub.y || y > sub.y + sub.h).toBe(true);
  });

  it('folds >=4 ended siblings into one "+N done" stack, and expanding shows them again', () => {
    const nodes = [lead({ id: 'l' }), ...Array.from({ length: 5 }, (_, i) => node({ id: `e${i}`, parentId: 'l', status: 'ended', startedAt: i })), node({ id: 'live', parentId: 'l', status: 'working', startedAt: 9 })];
    const closed = lay(nodes).layout;
    const fold = closed.nodes.find((n) => n.pseudo === 'fold')!;
    expect(fold).toMatchObject({ id: 'fold:l', count: 5, open: false, parent: 'l' });
    expect(closed.nodes.some((n) => n.id === 'e0')).toBe(false);
    expect(closed.edges.some((e) => e.to === 'fold:l')).toBe(true); // an edge whose child is folded goes to the stack
    const open = lay(nodes, { expanded: new Set(['l']) }).layout;
    expect(open.nodes.filter((n) => n.id.startsWith('e')).length).toBe(5);
    expect(open.nodes.find((n) => n.pseudo === 'fold')).toMatchObject({ open: true });
    expect(lay(nodes.slice(0, 4)).layout.nodes.some((n) => n.pseudo === 'fold')).toBe(false); // only 3 ended
  });

  it('does not fold an ended child that still has live descendants', () => {
    const nodes = [lead({ id: 'l' }), ...Array.from({ length: 5 }, (_, i) => node({ id: `e${i}`, parentId: 'l', status: 'ended' })), node({ id: 'kid', parentId: 'e0', status: 'working' })];
    expect(lay(nodes).layout.nodes.find((n) => n.pseudo === 'fold')?.count).toBe(4);
  });

  it('folds earlier router chats into "+N earlier chats" and expands them in the band', () => {
    const rc = (i: number) => node({ id: `rc${i}`, role: 'chat', groupKey: null, projectId: null, project: null, startedAt: i });
    const nodes = [SA, rc(1), rc(2), rc(3), ...project('oino', 1)];
    const closed = lay(nodes).layout;
    expect(closed.nodes.find((n) => n.pseudo === 'earlier')).toMatchObject({ count: 3, open: false });
    expect(closed.nodes.some((n) => n.id === 'rc1')).toBe(false);
    const open = lay(nodes, { expanded: new Set(['earlier']) }).layout;
    expect(open.nodes.filter((n) => n.id.startsWith('rc')).length).toBe(3);
    expect(pairwiseDisjoint(open.nodes)).toBe(true);
    expect(open.edges.some((e) => e.to.startsWith('rc'))).toBe(false);
  });

  it('dims the ancestors a filter kept: nodes are in the visible set and their edges are marked dimmed', () => {
    const nodes = [lead({ id: 'l', status: 'idle' }), node({ id: 'a', parentId: 'l', status: 'working' }), node({ id: 'b', parentId: 'l', status: 'ended' })];
    const { layout, tree } = lay(nodes, { filter: { ...F, statuses: ['working'] } });
    expect(layout.nodes.map((n) => n.id).sort()).toEqual(['a', 'l']);
    expect(filterTree(tree, { ...F, statuses: ['working'] }).dimmed.has('l')).toBe(true);
    const lone = lay([SA, ...project('oino', 1)], { filter: { ...F, statuses: ['stalled'] } }).layout;
    expect(lone.groups).toHaveLength(0);
    expect(lone.hiddenGroups).toBe(1);
    expect(lone.edges.every((e) => e.dimmed || e.from !== 'sa')).toBe(true);
  });

  it('a collapsed group keeps only its header and the trunk lands on its top edge', () => {
    const { layout } = lay(full, { collapsed: new Set(['oino']) });
    const g = layout.groups.find((x) => x.key === 'oino')!;
    expect(g.collapsed).toBe(true); expect(g.h).toBe(44);
    expect(layout.nodes.some((n) => n.group === 'oino')).toBe(false);
    const t = layout.edges.find((e) => e.from === 'sa' && e.to === 'lead-oino' && e.kind === 'base')!;
    expect(points(t.d).pop()![1]).toBe(g.y);
  });

  it('chats sit in a grid of at most 4 (detailed) or 6 (compact) columns under a Chats caption', () => {
    const chats = Array.from({ length: 9 }, (_, i) => node({ id: `c${i}`, role: 'chat', groupKey: 'oino', startedAt: 50 + i }));
    const d = lay([...project('oino', 1), ...chats]).layout;
    const rowsD = new Set(d.nodes.filter((n) => n.id.startsWith('c')).map((n) => n.y));
    expect(rowsD.size).toBe(3);
    expect(d.groups[0]!.chatsLabel).not.toBeNull();
    const c = lay([...project('oino', 1), ...chats], { density: 'compact' }).layout;
    expect(new Set(c.nodes.filter((n) => n.id.startsWith('c')).map((n) => n.y)).size).toBe(2);
    expect(lay(chats.map((n) => ({ ...n, groupKey: 'other', projectId: null }))).layout.groups[0]!.chatsLabel).toBeNull();
  });

  it('is deterministic and does not depend on node status for order or position', () => {
    const a = lay(full).layout;
    const flipped = lay(full.map((n) => (n.status === 'working' ? { ...n, status: 'idle' as const } : n))).layout;
    expect(flipped.nodes.map((n) => [n.id, n.x, n.y])).toEqual(a.nodes.map((n) => [n.id, n.x, n.y]));
  });

  it('lays out 300 nodes in well under a second, without overlaps', () => {
    const many: FleetNode[] = [SA];
    const gs = Array.from({ length: 12 }, (_, i) => ({ key: `p${i}`, name: `P${String(i).padStart(2, '0')}`, hasBoard: true }));
    gs.forEach((g) => many.push(...project(g.key, 23).map((n) => ({ ...n, project: g.name }))), ...[]);
    const tree = buildTree(makeFleet({ nodes: many, groups: gs, superagentId: 'sa', rev: 'x' }));
    const t0 = performance.now();
    const layout = layoutFleet(tree, F, 'detailed', new Set(), 2400);
    expect(performance.now() - t0).toBeLessThan(1000);
    expect(layout.nodes.length).toBe(many.length);
    expect(pairwiseDisjoint(layout.nodes)).toBe(true);
    expect(pairwiseDisjoint(layout.groups)).toBe(true);
  });

  it('navigates in layout order and finds the next node needing the owner', () => {
    const { layout, tree } = lay([...project('oino', 2), ...project('web', 1).map((n, i) => (i === 0 ? { ...n, status: 'waiting' as const } : n))]);
    const nav = navOf(layout);
    expect(nav.kids.get('lead-oino')).toEqual(['oino-0', 'oino-1']);
    expect(nav.parent.get('oino-1')).toBe('lead-oino');
    expect(nextNeedingYou(tree, layout)).toBe('lead-web');
    expect(nextNeedingYou(tree, layout, 'lead-web')).toBe('lead-web');
  });
});

describe('view maths', () => {
  it('fits everything at scale <= 1 with no floor, centred', () => {
    const big = fitView({ x: 0, y: 0, w: 8000, h: 3000 }, { width: 1000, height: 700 });
    expect(big.scale).toBeLessThan(0.3); expect(big.scale).toBeGreaterThanOrEqual(MIN_SCALE);
    expect(8000 * big.scale + 2 * big.x).toBeCloseTo(1000, 0);
    expect(fitView({ x: 0, y: 0, w: 100, h: 100 }, { width: 1000, height: 700 }).scale).toBe(1);
    expect(fitView({ w: 0, h: 0 }, { width: 10, height: 10 })).toEqual(RESET_VIEW);
    const off = fitView({ x: 64, y: 0, w: 1000, h: 400 }, { width: 1000, height: 700 });
    expect(64 * off.scale + off.x).toBeGreaterThanOrEqual(0);
  });
  it('zooms around a point and clamps to 0.2-2', () => {
    const v = zoomAt({ x: 0, y: 0, scale: 1 }, 2, 100, 50);
    expect(v).toEqual({ scale: 2, x: -100, y: -50 });
    expect(zoomAt(v, 100, 0, 0).scale).toBe(MAX_SCALE); expect(zoomAt(v, 0.0001, 0, 0).scale).toBe(MIN_SCALE);
  });
  it('level of detail: <.55 compact, <.35 dots', () => {
    expect([1, 0.55, 0.54, 0.35, 0.34, 0.2].map(lodOf)).toEqual([0, 0, 1, 1, 2, 2]);
  });
  it('culling rect grows the viewport by 25% on each side; pan-to-reveal leaves a visible node alone', () => {
    expect(worldRect({ x: 0, y: 0, scale: 1 }, { width: 1000, height: 400 })).toEqual({ x: -250, y: -100, w: 1500, h: 600 });
    const view = { x: 0, y: 0, scale: 1 };
    expect(panToReveal(view, { x: 100, y: 100, w: 100, h: 50 }, { width: 1000, height: 500 })).toBeNull();
    expect(panToReveal(view, { x: 3000, y: 100, w: 100, h: 50 }, { width: 1000, height: 500 })).toEqual({ scale: 1, x: 500 - 3050, y: 250 - 125 });
    expect(panToReveal(view, { x: 900, y: 100, w: 100, h: 50 }, { width: 1000, height: 500 }, { right: 200 })).not.toBeNull(); // under the peek pane
  });
});
