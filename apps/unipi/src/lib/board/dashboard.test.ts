import { describe, expect, it } from 'vitest';
import { activityLine, clusterActivity, dashStats, dayGroups, formatDuration, greeting, groupInbox, inboxCounts, median, relativeAgo, sortProjects, stepOf, summaryLine, throughput, waitAge } from './dashboard';
import { project, task } from './testing';
import type { DashboardData, DashboardInboxItem } from './api';

const NOW = Date.parse('2026-10-10T12:00:00Z');
const item = (slug: string, id: string, status: string, hoursAgo: number): DashboardInboxItem => ({
  slug,
  project: slug.toUpperCase(),
  task: task({ id, status }),
  waitingSince: new Date(NOW - hoursAgo * 3600_000).toISOString(),
  excerpt: null,
});

describe('dashboard logic', () => {
  it('formats ages, durations, greetings, medians', () => {
    expect(waitAge(new Date(NOW - 5 * 60_000).toISOString(), NOW)).toEqual({ label: '5m', old: false });
    expect(waitAge(new Date(NOW - 3 * 3600_000).toISOString(), NOW)).toEqual({ label: '3h', old: false });
    expect(waitAge(new Date(NOW - 50 * 3600_000).toISOString(), NOW)).toEqual({ label: '2d', old: true });
    expect(relativeAgo(null, NOW)).toBe('—');
    expect(relativeAgo(new Date(NOW - 10_000).toISOString(), NOW)).toBe('just now');
    expect(formatDuration(0)).toBe('0s');
    expect(formatDuration(45)).toBe('45s');
    expect(formatDuration(53760)).toBe('14h 56m');
    expect(formatDuration(2 * 86400 + 3 * 3600)).toBe('2d 3h');
    expect(greeting(8)).toBe('Good morning');
    expect(greeting(15)).toBe('Good afternoon');
    expect(greeting(21)).toBe('Good evening');
    expect(median([])).toBe(0);
    expect(median([3, 1, 2])).toBe(2);
    expect(median([1, 2, 3, 4])).toBe(3);
  });

  it('counts and summarises the inbox', () => {
    const items = [item('a', 'A-1', 'in_review', 1), item('a', 'A-2', 'blocked', 2)];
    expect(inboxCounts(items)).toEqual({ all: 2, review: 1, blocked: 1 });
    expect(summaryLine(inboxCounts(items), 1)).toBe('1 review and 1 blocked task waiting · 1 agent running');
    expect(summaryLine({ all: 0, review: 0, blocked: 0 }, 0)).toBe('All clear — nothing waiting on you.');
  });

  it('groups by project (oldest first), caps 3 per group and 10 overall, honours collapse/expand/all', () => {
    const items = [
      ...Array.from({ length: 5 }, (_, i) => item('new', `N-${i}`, 'in_review', 1)),
      ...Array.from({ length: 9 }, (_, i) => item('old', `O-${i}`, 'blocked', 30)),
    ];
    const groups = groupInbox(items, 'all', { collapsed: [], expanded: [], all: false });
    expect(groups.map((g) => g.slug)).toEqual(['old', 'new']);
    expect(groups.map((g) => g.visible.length)).toEqual([3, 3]);
    expect(groups[0]!.hidden).toBe(6);
    const expanded = groupInbox(items, 'all', { collapsed: [], expanded: ['old'], all: false });
    expect(expanded[0]!.visible.length).toBe(9);
    expect(expanded[1]!.visible.length).toBe(1); // 10-row budget
    const collapsed = groupInbox(items, 'all', { collapsed: ['old'], expanded: [], all: false });
    expect(collapsed[0]!.visible.length).toBe(0);
    expect(collapsed[0]!.collapsed).toBe(true);
    expect(groupInbox(items, 'review', { collapsed: [], expanded: [], all: true }).map((g) => g.visible.length)).toEqual([5]);
  });

  it('builds 14 days of throughput and stats', () => {
    const doneAt = [new Date(NOW - 3600_000).toISOString(), new Date(NOW - 3 * 86400_000).toISOString(), new Date(NOW - 20 * 86400_000).toISOString()];
    const days = throughput(doneAt, NOW);
    expect(days).toHaveLength(14);
    expect(days.at(-1)!.today).toBe(true);
    expect(days.reduce((sum, d) => sum + d.count, 0)).toBe(2);
    const data = { doneAt, reviewWaits: [60, 120], projects: [{ slug: 'a', review: 1, blocked: 2, ready: 0, lastActivity: null, problems: 1 }] } as unknown as DashboardData;
    expect(dashStats(data, NOW)).toEqual({ done7d: 2, medianWait: 90, hasWaits: true, blocked: 2, repairs: 1 });
  });

  it('reads activity verbs and clusters a task\'s burst into one row', () => {
    expect(stepOf('finished: did it')).toEqual({ label: 'finished', status: 'in_review' });
    expect(stepOf('moved todo → in_progress')).toEqual({ label: 'in_progress', status: 'in_progress' });
    expect(stepOf('created in backlog')).toEqual({ label: 'created', status: 'backlog' });
    expect(stepOf('started (pid 4 on oi)')).toEqual({ label: 'started', status: 'in_progress' });
    const at = (m: number) => new Date(NOW - m * 60_000).toISOString();
    const rows = clusterActivity([
      { slug: 'a', project: 'A', taskId: 'A-1', title: 't', at: at(1), actor: 'agent', session: null, text: 'finished: ok' },
      { slug: 'a', project: 'A', taskId: 'A-1', title: 't', at: at(3), actor: 'agent', session: null, text: 'started (pid 1 on x)' },
      { slug: 'a', project: 'A', taskId: 'A-2', title: 'u', at: at(4), actor: 'user', session: null, text: 'edited body' },
      { slug: 'a', project: 'A', taskId: 'A-2', title: 'u', at: at(5), actor: 'user', session: null, text: 'edited title' },
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]!.steps.map((s) => s.label)).toEqual(['started', 'finished']);
    expect(rows[1]!.repeats).toBe(2);
    expect(dayGroups(rows, NOW)[0]!.label).toBe('Today');
    expect(activityLine('![shot](att:x)\n\nhello\nworld')).toBe('hello');
  });

  it('sorts projects by attention, recency or name and drops archived ones', () => {
    const list = [project({ slug: 'b', name: 'Beta' }), project({ slug: 'a', name: 'Alpha', running: 1 }), project({ slug: 'z', name: 'Zed', archived: true })];
    const dash = new Map([
      ['b', { slug: 'b', review: 1, blocked: 1, ready: 0, lastActivity: '2026-10-01T00:00:00Z', problems: 0 }],
      ['a', { slug: 'a', review: 0, blocked: 0, ready: 0, lastActivity: '2026-10-09T00:00:00Z', problems: 0 }],
    ]);
    expect(sortProjects(list, dash, 'attention').map((p) => p.slug)).toEqual(['b', 'a']);
    expect(sortProjects(list, dash, 'recent').map((p) => p.slug)).toEqual(['a', 'b']);
    expect(sortProjects(list, dash, 'name').map((p) => p.slug)).toEqual(['a', 'b']);
  });
});
