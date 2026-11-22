import { describe, expect, it } from 'vitest';
import { cycleSessionModel, cycleToast, nextKey } from './cycleModel';
import { curatedFirst, defaultSuggestedKeys, groupedModels, routeLabel, startupKey } from './suggested';
import { parseFusionCurated } from '../../lib/model/fusion';
import { cleanHttpStatus } from '../../lib/model/availability';
import { toasts } from '../../lib/toasts';
import type { SessionHandle } from '../../lib/session/store';

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
    expect(routeLabel({ provider: 'gateway', id: 'cc/claude-opus-5-5' })).toBe('gateway · cc');
    expect(routeLabel({ provider: 'openai', id: 'gpt-5' })).toBe('openai');
    expect(startupKey('openai', 'gpt-5')).toBe('openai/gpt-5');
    expect(startupKey(undefined, 'a/b')).toBe('a/b');
    expect(startupKey(undefined, undefined)).toBeUndefined();
  });

  it('D-07 / F-04: large catalogue without preset gets a deduplicated Suggested group and cycles with a toast every press', async () => {
    const big = [
      { provider: 'oino', id: 'ollamacloud/deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash' },
      { provider: 'oino', id: 'deepseek/deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash' },
      { provider: 'oino', id: 'cc/claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
      { provider: 'oino', id: 'cc/claude-opus-4-1', name: 'Claude Opus 4.1' },
      { provider: 'oino', id: 'oa/gpt-5', name: 'GPT-5' },
      { provider: 'oino', id: 'ag/gemini-2.5-pro', name: 'Gemini 2.5 Pro' },
      { provider: 'oino', id: 'cc/claude-haiku-4-5', name: 'Claude Haiku 4.5' },
      { provider: 'oino', id: 'qwen/qwen3-coder', name: 'Qwen3 Coder' },
    ];
    const keys = defaultSuggestedKeys(big, 'oino/ollamacloud/deepseek-v4.1-flash');
    expect(keys.length).toBeGreaterThanOrEqual(4);
    expect(keys).not.toContain('oino/deepseek/deepseek-v4.1-flash'); // duplicate display name dropped
    const g = groupedModels(big, undefined, { kind: 'single', key: 'oino/ollamacloud/deepseek-v4.1-flash' }, '');
    expect(g.suggested.length).toBeGreaterThanOrEqual(4);
    expect(cleanHttpStatus('the key was rejected (HTTP 401 401 Unauthorized)')).toBe('the key was rejected (HTTP 401 Unauthorized)');

    toasts().clear();
    let current = { provider: 'oino', id: 'ollamacloud/deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash' };
    const fake = {
      key: 'h/s-cycle',
      hostId: '',
      state: { get model() { return current; } },
      async cmd(c: Record<string, unknown>) {
        if (c.type === 'get_available_models') return { models: big };
        if (c.type === 'set_model') current = big.find((m) => m.provider === c.provider && m.id === c.modelId) ?? current;
        return {};
      },
    } as unknown as SessionHandle;
    // Rapidly fire 3 times concurrently — all 3 must step and emit distinct toasts.
    const [t1, t2, t3] = await Promise.all([cycleSessionModel(fake), cycleSessionModel(fake), cycleSessionModel(fake)]);
    expect(t1).toMatch(/^Model → /);
    expect(t2).toMatch(/^Model → /);
    expect(t3).toMatch(/^Model → /);
    expect(new Set([t1, t2, t3]).size).toBe(3);
  });
});
