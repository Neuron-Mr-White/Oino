import { afterEach, describe, expect, it } from 'vitest';
import type { SessionCard } from '../host/client';
import {
  activeFilterCount,
  createdAt,
  createdFromFile,
  DEFAULT_VIEW,
  listView,
  matchCard,
  normalizeView,
  rangeBounds,
  resetListFilters,
  resetListViews,
  setListView,
  statusOf,
  viewGroups,
  type CardFacts,
  type ListView,
} from './listView';

const NOW = new Date(2026, 10, 15, 12, 0, 0).getTime();
const DAY = 86_400_000;
const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({ session_id: id, name: `S ${id}`, cwd: '/home/you/p/oino', run_state: 'idle', updated_at: NOW - 60_000, ...over });
const idle: CardFacts = { state: 'idle' };
const factsOf = (c: SessionCard): CardFacts => ({ state: c.run_state === 'working' ? 'working' : c.needs_you ? 'needs-you' : 'idle' });
const view = (over: Partial<ListView> = {}): ListView => ({ ...DEFAULT_VIEW, ...over, filters: { ...DEFAULT_VIEW.filters, ...over.filters } });

afterEach(() => resetListViews());

describe('listView facets', () => {
  it('reads the created time from the session file name, else last activity', () => {
    expect(createdFromFile('/x/2026-10-26T03-00-07-623Z_abc.jsonl')).toBe(Date.parse('2026-10-26T03:00:07.623Z'));
    expect(createdFromFile('/x/odd.jsonl')).toBeUndefined();
    expect(createdAt(card('a', { file: '/s/2026-01-02T00-00-00-000Z_a.jsonl' }))).toBe(Date.parse('2026-01-02T00:00:00.000Z'));
    expect(createdAt(card('b', { created_at: 42 } as Partial<SessionCard>))).toBe(42);
    expect(createdAt(card('c'))).toBe(NOW - 60_000);
  });

  it('maps live states and cron outcomes onto the Status facet', () => {
    expect(statusOf({ state: 'working' })).toBe('running');
    expect(statusOf({ state: 'needs-you', cronStatus: 'failed' })).toBe('waiting');
    expect(statusOf({ state: 'idle', cronStatus: 'timeout' })).toBe('timedout');
    expect(statusOf({ state: 'idle', cronStatus: 'failed' })).toBe('failed');
    expect(statusOf({ state: 'crashed' })).toBe('failed');
    expect(statusOf({ state: 'taken-over' })).toBe('ready');
  });

  it('time presets and custom inclusive day ranges', () => {
    const [from] = rangeBounds({ preset: 'today' }, NOW);
    expect(from).toBe(new Date(2026, 10, 15).getTime());
    expect(rangeBounds({ preset: 'week' }, NOW)[0]).toBe(NOW - 7 * DAY);
    const [a, b] = rangeBounds({ preset: 'custom', from: '2026-11-01', to: '2026-11-03' }, NOW);
    expect(a).toBe(new Date(2026, 10, 1).getTime());
    expect(b).toBe(new Date(2026, 10, 4).getTime());
  });

  it('counts active facets; Unarchived alone is the default', () => {
    expect(activeFilterCount(DEFAULT_VIEW.filters)).toBe(0);
    expect(activeFilterCount({ ...DEFAULT_VIEW.filters, status: ['running'], archived: [] })).toBe(2);
  });
});

describe('matchCard', () => {
  const f = DEFAULT_VIEW.filters;
  it('hides archived chats by default and shows only them under Archived', () => {
    expect(matchCard(card('a'), { ...idle, archived: true }, f, NOW)).toBe(false);
    expect(matchCard(card('a'), idle, f, NOW)).toBe(true);
    expect(matchCard(card('a'), { ...idle, archived: true }, { ...f, archived: ['archived'] }, NOW)).toBe(true);
    expect(matchCard(card('a'), idle, { ...f, archived: ['archived'] }, NOW)).toBe(false);
    expect(matchCard(card('a'), idle, { ...f, archived: [] }, NOW)).toBe(true);
  });

  it('filters by workspace (No workspace = ""), automation, model and time', () => {
    expect(matchCard(card('a', { cwd: '/home/you' }), idle, { ...f, workspaces: [''] }, NOW)).toBe(true);
    expect(matchCard(card('a'), idle, { ...f, workspaces: [''] }, NOW)).toBe(false);
    expect(matchCard(card('a', { source: 'cron' }), idle, { ...f, automation: ['cron'] }, NOW)).toBe(true);
    expect(matchCard(card('a'), idle, { ...f, automation: ['cron'] }, NOW)).toBe(false);
    expect(matchCard(card('a', { model: 'anthropic/claude' }), idle, { ...f, models: ['anthropic/claude'] }, NOW)).toBe(true);
    expect(matchCard(card('a'), { ...idle, model: 'x' }, { ...f, models: [''] }, NOW)).toBe(false);
    expect(matchCard(card('a', { updated_at: NOW - 20 * DAY }), idle, { ...f, updated: { preset: '2weeks' } }, NOW)).toBe(false);
    expect(matchCard(card('a', { updated_at: NOW - 20 * DAY }), idle, { ...f, updated: { preset: 'month' } }, NOW)).toBe(true);
  });
});

describe('viewGroups', () => {
  const cards = [
    card('a', { cwd: '/p/one', updated_at: NOW - 1000 }),
    card('b', { cwd: '/p/two', updated_at: NOW - 500, run_state: 'working' }),
    card('c', { cwd: '/p/one', updated_at: NOW - 100, source: 'cron', name: 'Alpha' }),
    card('d', { cwd: '/p/two', updated_at: NOW - 50, needs_you: true }),
    card('p', { cwd: '/p/one', updated_at: NOW }),
  ];
  const meta = { p: { pinned: true } };

  it('Project grouping keeps Pinned on top and the Z-9 order inside', () => {
    const g = viewGroups(cards, meta, view(), factsOf, NOW);
    expect(g.pinned.map((c) => c.session_id)).toEqual(['p']);
    expect(g.groups.map((x) => x.label)).toEqual(['two', 'one']);
    expect(g.groups[0]!.cards.map((c) => c.session_id)).toEqual(['b', 'd']);
    expect(g.groups[0]!.cwd).toBe('/p/two');
    expect(g.total).toBe(5);
  });

  it('Status / Automation / None groupings and the Name sort', () => {
    const s = viewGroups(cards, {}, view({ group: 'status' }), factsOf, NOW);
    expect(s.groups.map((x) => x.label)).toEqual(['Running', 'Waiting for you', 'Ready']);
    const a = viewGroups(cards, {}, view({ group: 'automation' }), factsOf, NOW);
    expect(a.groups.map((x) => [x.label, x.cards.length])).toEqual([
      ['Automation', 1],
      ['Not automation', 4],
    ]);
    const n = viewGroups(cards, {}, view({ group: 'none', sort: 'name' }), factsOf, NOW);
    expect(n.groups).toHaveLength(1);
    expect(n.groups[0]!.cards.map((c) => c.session_id)).toEqual(['c', 'a', 'b', 'd', 'p']);
  });

  it('filters before grouping (empty result → total 0)', () => {
    const g = viewGroups(cards, meta, view({ filters: { ...DEFAULT_VIEW.filters, status: ['failed'] } }), factsOf, NOW);
    expect(g.total).toBe(0);
    expect(g.groups).toEqual([]);
    expect(g.pinned).toEqual([]);
  });

  it('stays fast on 2,000 cards', () => {
    const many = Array.from({ length: 2000 }, (_, i) => card(`s${i}`, { cwd: `/p/${i % 40}`, updated_at: NOW - i * 1000, source: i % 7 ? undefined : 'cron' }));
    const t0 = performance.now();
    for (let i = 0; i < 5; i++) viewGroups(many, {}, view({ filters: { ...DEFAULT_VIEW.filters, automation: ['manual'] } }), factsOf, NOW);
    expect((performance.now() - t0) / 5).toBeLessThan(60);
  });
});

describe('the persisted per-host store', () => {
  it('patches one host, persists, and resets filters', () => {
    setListView('h1', { sort: 'name', filters: { status: ['running'] }, props: { model: true } });
    expect(listView('h1').sort).toBe('name');
    expect(listView('h1').props.model).toBe(true);
    expect(listView('h1').props.updated).toBe(true);
    expect(listView('h2')).toBe(DEFAULT_VIEW);
    const stored = JSON.parse(localStorage.getItem('unipi.sidebar.view.v1')!);
    expect(stored.h1.filters.status).toEqual(['running']);
    setListView('h1', { filters: { created: { preset: 'today' } } });
    resetListFilters('h1');
    expect(listView('h1').filters).toEqual(DEFAULT_VIEW.filters);
    expect(listView('h1').sort).toBe('name');
  });

  it('normalizes junk storage', () => {
    const v = normalizeView({ sort: 'nope', group: 'status', filters: { status: ['running', 'x'], created: { preset: 'bogus' } }, compact: 'yes' });
    expect(v.sort).toBe('updated');
    expect(v.group).toBe('status');
    expect(v.filters.status).toEqual(['running']);
    expect(v.filters.created).toBeUndefined();
    expect(v.filters.archived).toEqual(['unarchived']);
    expect(v.compact).toBe(false);
  });
});
