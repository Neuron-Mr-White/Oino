// @vitest-environment jsdom
/**
 * The subagent tree's status filter (owner 2026-11-06): it opens on "In
 * progress" — the runs that are still going — not on "All". A chat without a
 * running subagent keeps that default and shows the empty state; the tree
 * never switches itself to "All".
 */
import { describe, expect, it } from 'vitest';
import { DEFAULT_FILTER, filterOf, filteredRuns, hasFilter, type SubRun } from './SubagentTree';

const run = (id: string, status: string): SubRun => ({ id, title: id, status });

describe('the subagent tree filter', () => {
  it('opens on "In progress" (running)', () => {
    const runs = [run('a', 'running'), run('b', 'running'), run('c', 'completed'), run('d', 'completed')];
    expect(DEFAULT_FILTER).toBe('running');
    expect(hasFilter(runs)).toBe(true);
    expect(filterOf('h', 's', runs)).toBe('running');
    expect(filteredRuns('h', 's', runs).map((r) => r.id)).toEqual(['a', 'b']);
  });

  it('keeps "In progress" when nothing is running (the empty state, not "All")', () => {
    const runs = [run('a', 'completed'), run('b', 'completed'), run('c', 'failed'), run('d', 'completed')];
    expect(filterOf('h', 'empty', runs)).toBe('running');
    expect(filteredRuns('h', 'empty', runs)).toEqual([]);
  });

  it('a tree without a filter row shows every run', () => {
    const runs = [run('a', 'completed'), run('b', 'running')];
    expect(hasFilter(runs)).toBe(false);
    expect(filterOf('h', 'small', runs)).toBeUndefined();
    expect(filteredRuns('h', 'small', runs).map((r) => r.id)).toEqual(['a', 'b']);
  });
});
