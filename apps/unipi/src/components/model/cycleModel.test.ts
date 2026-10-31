import { describe, expect, it } from 'vitest';
import { cycleToast, nextKey } from './cycleModel';
import { curatedFirst, groupedModels, routeLabel, startupKey } from './suggested';
import { parseFusionCurated } from '../../lib/model/fusion';

describe('model cycle (P0-10)', () => {
  it('steps the list and wraps; an unknown current starts at the top', () => {
    expect(nextKey(['a', 'b', 'c'], 'a')).toBe('b');
    expect(nextKey(['a', 'b', 'c'], 'c')).toBe('a');
    expect(nextKey(['a', 'b'], 'zzz')).toBe('a');
    expect(nextKey([], 'a')).toBeUndefined();
    expect(cycleToast('GPT-5')).toBe('Model → GPT-5');
  });
});

describe('curated payload + grouping (P0-10)', () => {
  const curated = parseFusionCurated({
    lead: [{ key: 'a/x', name: 'X' }, { key: 'gone/q', name: 'q' }],
    sidekick: [{ key: 'b/y', name: 'Y' }],
    ranked: [{ key: 'b/y', name: 'Y' }, { key: 'a/x', name: 'X' }, { key: 'b/y', name: 'dup' }],
    default: { lead: 'a/x' },
    effort: { 'a/x': 'high', bad: 3 },
  })!;
  const models = [
    { provider: 'a', id: 'x', name: 'X' },
    { provider: 'b', id: 'y', name: 'Y' },
    { provider: 'c', id: 'z', name: 'Z' },
  ];

  it('parses leniently (dedup, string efforts only, absent → undefined)', () => {
    expect(curated.ranked.map((e) => e.key)).toEqual(['b/y', 'a/x']);
    expect(curated.effort).toEqual({ 'a/x': 'high' });
    expect(curated.default).toEqual({ lead: 'a/x' });
    expect(parseFusionCurated(undefined)).toBeUndefined();
  });

  it('suggested = ranked with roles + effort; rest = the catalogue minus them', () => {
    const g = groupedModels(models, curated, { kind: 'single', key: 'c/z' }, '');
    expect(g.suggested.map((r) => [r.model.id, r.lead, r.sidekick, r.effort])).toEqual([
      ['y', false, true, undefined],
      ['x', true, false, 'high'],
    ]);
    expect(g.rest.map((m) => m.id)).toEqual(['z']);
    expect(groupedModels(models, undefined, undefined, '').suggested).toEqual([]);
  });

  it('columns put the curated list first', () => {
    expect(curatedFirst(models, ['b/y', 'missing']).curated.map((m) => m.id)).toEqual(['y']);
  });

  it('route label keeps routers apart without a raw id; startup key from pi defaults', () => {
    expect(routeLabel({ provider: 'omniroute', id: 'cc/claude-opus-5-5' })).toBe('omniroute · cc');
    expect(routeLabel({ provider: 'openai', id: 'gpt-5' })).toBe('openai');
    expect(startupKey('openai', 'gpt-5')).toBe('openai/gpt-5');
    expect(startupKey(undefined, 'a/b')).toBe('a/b');
    expect(startupKey(undefined, undefined)).toBeUndefined();
  });
});
