import { describe, expect, it } from 'vitest';
import { claimState, dependentsOf, reorderPlan, splitReport, statusMoves, statusReport, canReorder } from './report';
import { PERMISSIVE_RULES, type Rules } from './api';
import { task } from './testing';
import { uploadAll, MAX_UPLOAD_BYTES } from './attach';

const rules: Rules = {
  ...PERMISSIVE_RULES,
  allowedMoves: { in_review: ['done', 'todo', 'backlog', 'archived'], todo: ['backlog', 'cancelled'] },
  commentRequired: {},
};

describe('task detail helpers', () => {
  it('splitReport labels status-changing activity like the web', () => {
    expect(splitReport('finished: ## Summary\nok')).toEqual({ label: 'Finished', body: '## Summary\nok' });
    expect(splitReport('released to blocked: need keys')).toEqual({ label: 'Blocked', body: 'need keys' });
    expect(splitReport('rework: try again')).toEqual({ label: 'Rework', body: 'try again' });
    expect(splitReport('released to todo: stale')).toEqual({ label: 'Released', body: 'stale' });
    expect(splitReport('just a note')).toEqual({ label: null, body: 'just a note' });
  });

  it('statusReport falls back to the blocked reason on older daemons', () => {
    const blocked = task({ id: 'A', status: 'blocked', blockedReason: { text: 'why', actor: 'agent', at: 't' } });
    expect(statusReport(blocked)).toEqual({ kind: 'blocked', text: 'why', actor: 'agent', at: 't' });
    const review = task({ id: 'B', status: 'in_review', statusReport: { kind: 'review', text: 'done', actor: 'agent', at: 't' } });
    expect(statusReport(review)?.kind).toBe('review');
    expect(statusReport(task({ id: 'C' }))).toBeUndefined();
  });

  it('statusMoves: allowed only, board order, never the current lane or Archive', () => {
    expect(statusMoves(rules, task({ id: 'A', status: 'in_review' }))).toEqual(['backlog', 'todo', 'done']);
    // Per-task allowedMoves (the daemon's, user actor) win over the rules table.
    expect(statusMoves(rules, task({ id: 'B', status: 'todo', allowedMoves: ['backlog'] }))).toEqual(['backlog']);
  });

  it('claimState: running / stale / unknown / free', () => {
    expect(claimState(task({ id: 'A' }))).toBe('free');
    expect(claimState(task({ id: 'B', run: { pid: 1 }, staleness: 'running' }))).toBe('running');
    expect(claimState(task({ id: 'C', run: { pid: 1 }, staleness: 'stale' }))).toBe('stale');
    expect(claimState(task({ id: 'D', run: { pid: 1 }, staleness: 'unknown' }))).toBe('unknown');
    expect(canReorder(task({ id: 'E', run: { pid: 1 } }))).toBe(false);
    expect(canReorder(task({ id: 'F', status: 'done' }))).toBe(false);
  });

  it('dependentsOf lists the tasks that run after a task', () => {
    const tasks = [task({ id: 'A' }), task({ id: 'B', deps: ['A'] }), task({ id: 'C', deps: ['B', 'A'] })];
    expect(dependentsOf(tasks, 'A').map((t) => t.id)).toEqual(['B', 'C']);
  });
});

describe('reorderPlan', () => {
  const lane = [
    task({ id: 'A', order: 1000 }),
    task({ id: 'B', order: 2000 }),
    task({ id: 'C', order: 3000, deps: ['B'] }), // B → C drawn as one block
    task({ id: 'D', order: 4000 }),
  ];

  it('single cards: before the previous block / after the next block / top / bottom', () => {
    expect(reorderPlan(lane, true, 'D', 'up')).toEqual([{ id: 'D', target: { before: 'B' } }]);
    expect(reorderPlan(lane, true, 'A', 'down')).toEqual([{ id: 'A', target: { afterPos: 'C' } }]);
    expect(reorderPlan(lane, true, 'D', 'top')).toEqual([{ id: 'D', target: { top: true } }]);
    expect(reorderPlan(lane, true, 'A', 'bottom')).toEqual([{ id: 'A', target: { bottom: true } }]);
  });

  it('edges are no-ops', () => {
    expect(reorderPlan(lane, true, 'A', 'up')).toEqual([]);
    expect(reorderPlan(lane, true, 'A', 'top')).toEqual([]);
    expect(reorderPlan(lane, true, 'D', 'down')).toEqual([]);
    expect(reorderPlan(lane, true, 'nope', 'down')).toEqual([]);
  });

  it('a chain moves as a block, members keep their order', () => {
    expect(reorderPlan(lane, true, 'C', 'up')).toEqual([
      { id: 'B', target: { before: 'A' } },
      { id: 'C', target: { afterPos: 'B' } },
    ]);
    expect(reorderPlan(lane, true, 'B', 'bottom')).toEqual([
      { id: 'B', target: { bottom: true } },
      { id: 'C', target: { afterPos: 'B' } },
    ]);
  });

  it('without chain grouping every card is its own block', () => {
    expect(reorderPlan(lane, false, 'C', 'up')).toEqual([{ id: 'C', target: { before: 'B' } }]);
  });
});

describe('uploads', () => {
  it('refuses files over 25 MB before sending them', async () => {
    const errors: string[] = [];
    const big = new File([new Uint8Array(8)], 'huge.mov', { type: 'video/quicktime' });
    Object.defineProperty(big, 'size', { value: MAX_UPLOAD_BYTES + 1 });
    const done = await uploadAll('pc1', 'p', 'A', [big], (message) => errors.push(message));
    expect(done).toEqual([]);
    expect(errors).toEqual(['huge.mov is larger than 25 MB']);
  });
});
