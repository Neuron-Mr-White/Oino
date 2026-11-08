import { describe, expect, it } from 'vitest';
import { dragBlocked, dropAllowed, indexAt, planDrop, snapBefore } from './dnd';
import { task } from './testing';
import { laneLabel, type Rules } from './api';

const RULES: Rules = {
  statuses: [],
  allowedMoves: { todo: ['backlog', 'cancelled'], in_review: ['done', 'todo'], in_progress: ['todo', 'backlog'], blocked: ['todo', 'backlog', 'done', 'cancelled'], backlog: ['todo'] },
  commentRequired: { in_progress: { todo: 'why the stale run is being released' } },
  final: ['done', 'cancelled', 'archived'],
};

describe('dnd', () => {
  it('blocks running and final cards, with a reason', () => {
    expect(dragBlocked(task({ id: 'A-1', run: { session: 's1', started: '' } }))).toMatch(/claimed by session s1/);
    expect(dragBlocked(task({ id: 'A-2', status: 'done' }))).toMatch(/done/);
    expect(dragBlocked(task({ id: 'A-3', status: 'todo' }))).toBeUndefined();
  });

  it('snaps a drop inside a chain to the chain start', () => {
    const items = [
      { chain: 'single' as const, task: { id: 'a' } },
      { chain: 'first' as const, task: { id: 'b' } },
      { chain: 'middle' as const, task: { id: 'c' } },
      { chain: 'last' as const, task: { id: 'd' } },
      { chain: 'single' as const, task: { id: 'e' } },
    ];
    expect(snapBefore(items, 0)).toBe('a');
    expect(snapBefore(items, 2)).toBe('b');
    expect(snapBefore(items, 3)).toBe('b');
    expect(snapBefore(items, 4)).toBe('e');
    expect(snapBefore(items, -1)).toBeNull();
  });

  it('finds the first card below the pointer midpoint', () => {
    const rects = [{ top: 0, height: 100 }, { top: 110, height: 100 }];
    expect(indexAt(rects, 10)).toBe(0);
    expect(indexAt(rects, 60)).toBe(1);
    expect(indexAt(rects, 300)).toBe(-1);
  });

  it('plans reorders, no-ops, refusals and moves', () => {
    const t = task({ id: 'T-2', status: 'todo' });
    const ids = ['T-1', 'T-2', 'T-3'];
    expect(planDrop(RULES, t, 'todo', 'T-3', ids, laneLabel)).toEqual({ kind: 'none' });
    expect(planDrop(RULES, t, 'todo', 'T-2', ids, laneLabel)).toEqual({ kind: 'none' });
    expect(planDrop(RULES, t, 'todo', 'T-1', ids, laneLabel)).toEqual({ kind: 'reorder', order: { before: 'T-1' } });
    expect(planDrop(RULES, t, 'todo', null, ids, laneLabel)).toEqual({ kind: 'reorder', order: { bottom: true } });
    expect(planDrop(RULES, t, 'done', null, ids, laneLabel)).toEqual({ kind: 'refused', message: "T-2 can't move from Todo to Done" });
    expect(planDrop(RULES, t, 'backlog', 'B-1', [], laneLabel)).toEqual({ kind: 'move', to: 'backlog', note: null, order: { before: 'B-1' } });
    const stale = task({ id: 'S-1', status: 'in_progress' });
    expect(planDrop(RULES, stale, 'todo', null, [], laneLabel)).toEqual({ kind: 'move', to: 'todo', note: 'why the stale run is being released', order: { bottom: true } });
  });

  it('marks allowed drop lanes', () => {
    const t = task({ id: 'T-1', status: 'in_review' });
    expect(dropAllowed(RULES, t, 'in_review')).toBe(true);
    expect(dropAllowed(RULES, t, 'done')).toBe(true);
    expect(dropAllowed(RULES, t, 'blocked')).toBe(false);
  });

  it('allows a blocked card to be dropped back in the backlog', () => {
    const blocked = task({ id: 'B-1', status: 'blocked' });
    expect(dropAllowed(RULES, blocked, 'backlog')).toBe(true);
    expect(dropAllowed(RULES, blocked, 'todo')).toBe(true);
    expect(planDrop(RULES, blocked, 'backlog', null, [], laneLabel)).toEqual({
      kind: 'move',
      to: 'backlog',
      note: null,
      order: { bottom: true },
    });
  });
});
