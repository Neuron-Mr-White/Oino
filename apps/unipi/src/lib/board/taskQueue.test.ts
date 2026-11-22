import { describe, expect, it } from 'vitest';
import { queueAfterLeave, queueNeighbour, queuePosition, snapshotQueue, taskBackTarget, type QueueRef } from './taskQueue';
import { phoneParent } from '../phoneNav';

const q = (...tasks: string[]): QueueRef[] => tasks.map((task) => ({ project: task.split('-')[0]!.toLowerCase(), task }));

describe('taskBackTarget (OIN-40)', () => {
  it('a task opened from Superagent goes back to Superagent', () => expect(taskBackTarget('superagent')).toBe('/superagent'));
  it('a task opened from the board (or anywhere else) goes back to the board', () => {
    expect(taskBackTarget(undefined)).toBe('/board');
    expect(taskBackTarget('board')).toBe('/board');
  });
  it('phone Back follows the origin and leaves the board table unchanged', () => {
    expect(phoneParent('/board/task/HP-3', '?from=superagent')).toBe('/superagent');
    expect(phoneParent('/board/task/HP-3', '')).toBe('/board');
    expect(phoneParent('/board/task/HP-3', '?from=board')).toBe('/board');
    expect(phoneParent('/board', '')).toBe('/chat');
  });
});

describe('queue snapshot, position and neighbours', () => {
  it('keeps the list order, drops duplicates and always contains the opened task', () => {
    expect(snapshotQueue(q('HP-3', 'OIN-12', 'HP-3'), { project: 'hp', task: 'HP-3' }).map((r) => r.task)).toEqual(['HP-3', 'OIN-12']);
    expect(snapshotQueue(q('HP-3'), { project: 'oin', task: 'OIN-12' }).map((r) => r.task)).toEqual(['OIN-12', 'HP-3']);
  });
  it('reports "3 of 12" style positions', () => {
    const items = q('A-1', 'A-2', 'A-3', 'A-4');
    expect(queuePosition(items, 'A-3')).toEqual({ index: 3, total: 4 });
    expect(queuePosition(items, 'B-1')).toBeUndefined();
  });
  it('finds prev / next without wrapping', () => {
    const items = q('A-1', 'A-2', 'A-3');
    expect(queueNeighbour(items, 'A-2', 1)?.task).toBe('A-3');
    expect(queueNeighbour(items, 'A-2', -1)?.task).toBe('A-1');
    expect(queueNeighbour(items, 'A-1', -1)).toBeUndefined();
    expect(queueNeighbour(items, 'A-3', 1)).toBeUndefined();
  });
});

describe('queueAfterLeave: what shows after Mark done / Rework', () => {
  it('shows the item that slid into the place of the one that left', () => {
    const out = queueAfterLeave(q('A-1', 'A-2', 'A-3'), 'A-2');
    expect(out.items.map((r) => r.task)).toEqual(['A-1', 'A-3']);
    expect(out.next?.task).toBe('A-3');
  });
  it('after the last item, falls back to the nearest earlier one still waiting', () => {
    const out = queueAfterLeave(q('A-1', 'A-2', 'A-3'), 'A-3');
    expect(out.next?.task).toBe('A-2');
  });
  it('is exhausted when the last remaining item leaves', () => {
    const out = queueAfterLeave(q('A-1'), 'A-1');
    expect(out.items).toEqual([]);
    expect(out.next).toBeUndefined();
  });
});
