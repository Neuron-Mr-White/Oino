import { describe, expect, it } from 'vitest';
import {
  DEFAULT_DISPLAY,
  depCandidates,
  excerptOf,
  EMPTY_FILTERS,
  filterCount,
  laneCount,
  laneLayout,
  laneTasks,
  mentionAt,
  mentionCandidates,
  searchTasks,
  visibleLanes,
  wouldCycle,
} from './view';
import { task } from './testing';

const tasks = [
  task({ id: 'U-1', title: 'Offline login', status: 'todo', priority: 'high', labels: ['auth'], deps: ['U-2'], waitingFor: ['U-2'], order: 2000 }),
  task({ id: 'U-2', title: 'Token cache', status: 'todo', priority: 'medium', labels: ['auth'], order: 1000 }),
  task({ id: 'U-3', title: 'Dark mode', status: 'todo', priority: 'low', labels: ['ui'], body: 'polish the settings screen', order: 500 }),
  task({ id: 'U-4', title: 'Crash on rotate', status: 'in_progress', priority: 'urgent', run: { session: 's1' } }),
  task({ id: 'U-5', title: 'Iroh upgrade', status: 'blocked' }),
  task({ id: 'U-6', title: 'Old idea', status: 'backlog' }),
  task({ id: 'U-7', title: 'Dropped', status: 'cancelled' }),
];

describe('lanes', () => {
  it('counts every task per lane regardless of filters', () => {
    expect(laneCount(tasks, 'todo')).toBe(3);
    expect(laneCount(tasks, 'done')).toBe(0);
  });
  it('orders a lane by stored order and filters by priority / label / running / waiting / text', () => {
    expect(laneTasks(tasks, 'todo', '', EMPTY_FILTERS).map((t) => t.id)).toEqual(['U-3', 'U-2', 'U-1']);
    expect(laneTasks(tasks, 'todo', '', { ...EMPTY_FILTERS, priorities: ['high'] }).map((t) => t.id)).toEqual(['U-1']);
    expect(laneTasks(tasks, 'todo', '', { ...EMPTY_FILTERS, labels: ['auth'] }).map((t) => t.id)).toEqual(['U-2', 'U-1']);
    expect(laneTasks(tasks, 'todo', '', { ...EMPTY_FILTERS, waiting: true }).map((t) => t.id)).toEqual(['U-1']);
    expect(laneTasks(tasks, 'in_progress', '', { ...EMPTY_FILTERS, running: true }).map((t) => t.id)).toEqual(['U-4']);
    expect(laneTasks(tasks, 'todo', 'settings', EMPTY_FILTERS).map((t) => t.id)).toEqual(['U-3']); // body
    expect(laneTasks(tasks, 'todo', 'u-2', EMPTY_FILTERS).map((t) => t.id)).toEqual(['U-2']); // id
    expect(laneTasks(tasks, 'todo', 'UI', EMPTY_FILTERS).map((t) => t.id)).toEqual(['U-3']); // label
    expect(filterCount({ priorities: ['high', 'low'], labels: ['x'], running: true, waiting: false })).toBe(4);
  });
  it('pulls same-lane dependency chains together (parents first)', () => {
    const layout = laneLayout(laneTasks(tasks, 'todo', '', EMPTY_FILTERS), true);
    expect(layout.map((item) => `${item.task.id}:${item.chain}`)).toEqual(['U-3:single', 'U-2:first', 'U-1:last']);
    expect(layout[2]!.parents).toEqual(['U-2']);
    expect(laneLayout(laneTasks(tasks, 'todo', '', EMPTY_FILTERS), false).every((item) => item.chain === 'single')).toBe(true);
  });
  it('shows backlog only with showBacklog, hides archived by default, honours scope', () => {
    expect(visibleLanes(DEFAULT_DISPLAY, false).map((l) => l.id)).toEqual(['todo', 'in_progress', 'blocked', 'in_review', 'done', 'cancelled']);
    expect(visibleLanes(DEFAULT_DISPLAY, true)[0]!.id).toBe('backlog');
    expect(visibleLanes({ ...DEFAULT_DISPLAY, hidden: ['cancelled'] }, false).map((l) => l.id)).toContain('archived');
    expect(visibleLanes(DEFAULT_DISPLAY, false, 'in_review').map((l) => l.id)).toEqual(['in_review']);
  });
});

describe('search', () => {
  it('ranks id/title hits before body/label hits, fuzzy only as a fallback', () => {
    expect(searchTasks(tasks, 'cache').map((t) => t.id)).toEqual(['U-2']);
    expect(searchTasks(tasks, 'settings').map((t) => t.id)).toEqual(['U-3']);
    expect(searchTasks(tasks, 'U-4')[0]!.id).toBe('U-4');
    expect(searchTasks(tasks, 'crsh rot').map((t) => t.id)).toEqual(['U-4']);
    expect(searchTasks(tasks, 'zzzz')).toEqual([]);
    expect(searchTasks(tasks, '')).toHaveLength(7);
  });
});

describe('dependencies + mentions', () => {
  it('excludes cancelled/archived, self and cycles', () => {
    const ids = depCandidates(tasks, '', new Set(), 'U-2').shown.map((t) => t.id);
    expect(ids).not.toContain('U-7');
    expect(ids).not.toContain('U-2');
    expect(ids).not.toContain('U-1'); // U-1 runs after U-2 → would cycle
    expect(wouldCycle(tasks, 'U-2', 'U-1')).toBe(true);
    expect(depCandidates(tasks, 'iroh', new Set(), undefined).shown.map((t) => t.id)).toEqual(['U-5']);
  });
  it('finds the @token before the caret', () => {
    expect(mentionAt('see @U-', 7)).toEqual({ start: 4, needle: 'U-' });
    expect(mentionAt('mail a@b', 8)).toBeNull();
    expect(mentionAt('@', 1)).toEqual({ start: 0, needle: '' });
  });
  it('ranks an exact / prefix id match above title matches', () => {
    const list = [
      task({ id: 'U-12', title: 'Filler', created: '2026-10-03T00:00:00Z' }),
      task({ id: 'U-27', title: 'U-1 relay smoke test', created: '2026-10-05T00:00:00Z' }),
      task({ id: 'U-1', title: 'Offline login', created: '2026-10-01T00:00:00Z' }),
    ];
    expect(mentionCandidates(list, 'U-1').map((t) => t.id)).toEqual(['U-1', 'U-12', 'U-27']);
    expect(mentionCandidates(list, '').map((t) => t.id)).toEqual(['U-27', 'U-12', 'U-1']);
  });
});

describe('excerptOf', () => {
  it('flattens markdown and shows attachments by name, not as raw att: links', () => {
    const t = task({ id: 'U-9', body: 'Body **text**\n\n![shot.png](att:U-9/3fd6-shot.png)\n[log.txt](att:U-9/02-log.txt)' });
    expect(excerptOf(t)).toBe('Body text shot.png log.txt');
  });
});
