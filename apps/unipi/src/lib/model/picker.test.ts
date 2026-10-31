/**
 * Model picker rules (WP-A14 acceptance): one search over the whole
 * catalogue and both Fusion columns, the current row first and checked, the
 * Vision/context badges, the one explicit apply action and the switch toast.
 */
import { describe, expect, it } from 'vitest';
import {
  columnModels,
  contextBadge,
  hasVision,
  modelKey,
  modelName,
  nextSinglePick,
  nowLabel,
  pairUnchanged,
  parseModels,
  selectionOf,
  shortModelName,
  singleAction,
  singleModels,
  switchToast,
  type ModelInfo,
} from './picker';

const models: ModelInfo[] = [
  { provider: 'anthropic', id: 'claude-sonnet-4-5', name: 'Anthropic: Claude Sonnet 4.5', input: ['text', 'image'], contextWindow: 200_000 },
  { provider: 'openai', id: 'gpt-5', name: 'GPT-5', input: ['text', 'image'], contextWindow: 400_000, reasoning: true },
  { provider: 'zai', id: 'glm-4.6', name: 'GLM-4.6', input: ['text'], contextWindow: 128_000 },
  { provider: 'google', id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', input: ['text', 'image'], contextWindow: 1_000_000 },
];

describe('parseModels', () => {
  it('reads {models} (the host reply) and drops rows without provider/id', () => {
    const parsed = parseModels({ models: [{ provider: 'a', id: 'b', name: 'B', reasoning: true, input: ['text', 'image'], contextWindow: 1000 }, { provider: 'x' }, null] });
    expect(parsed).toHaveLength(1);
    expect(parsed[0]).toMatchObject({ provider: 'a', id: 'b', name: 'B', reasoning: true, contextWindow: 1000 });
  });
  it('accepts a bare array and de-dupes by key', () => {
    expect(parseModels([{ provider: 'a', id: 'b' }, { provider: 'a', id: 'b' }])).toHaveLength(1);
  });
  it('survives anything else', () => {
    expect(parseModels(undefined)).toEqual([]);
    expect(parseModels('nope')).toEqual([]);
  });
});

describe('row labels and badges', () => {
  it('strips the provider prefix of "Anthropic: …" names but not other names', () => {
    expect(shortModelName('Anthropic: Claude Sonnet 4.5')).toBe('Claude Sonnet 4.5');
    expect(shortModelName('GPT-5')).toBe('GPT-5');
    expect(shortModelName(undefined)).toBe('');
  });
  it('falls back to the id when the snapshot carried no name', () => {
    expect(modelName({ provider: 'a', id: 'b' })).toBe('B');
    expect(modelName({ provider: 'a', id: 'b', name: '  ' })).toBe('B');
  });
  it('marks Vision models and formats the context window', () => {
    expect(hasVision(models[0]!)).toBe(true);
    expect(hasVision(models[2]!)).toBe(false);
    expect(contextBadge(models[0]!)).toBe('200k');
    expect(contextBadge(models[3]!)).toBe('1M');
    expect(contextBadge({ provider: 'a', id: 'b' })).toBeUndefined();
  });
  it('keys are provider/id', () => {
    expect(modelKey(models[0]!)).toBe('anthropic/claude-sonnet-4-5');
  });
});

describe('singleModels (one search over everything)', () => {
  it('puts the running model first, then the host order', () => {
    const rows = singleModels(models, { kind: 'single', key: 'zai/glm-4.6' }, '');
    expect(rows[0]!.id).toBe('glm-4.6');
    expect(rows.map((m) => m.id)).toEqual(['glm-4.6', 'claude-sonnet-4-5', 'gpt-5', 'gemini-2.5-pro']);
  });
  it('a search spans the whole catalogue, not just the top rows', () => {
    const rows = singleModels(models, { kind: 'single', key: 'anthropic/claude-sonnet-4-5' }, 'gem');
    expect(rows.map((m) => m.id)).toEqual(['gemini-2.5-pro']);
  });
  it('matches the provider/id sub-line too', () => {
    expect(singleModels(models, undefined, 'zai/').map((m) => m.id)).toEqual(['glm-4.6']);
  });
});

describe('columnModels (Fusion shares the search)', () => {
  it('the same query filters both columns identically', () => {
    const lead = columnModels(models, 'g');
    const sidekick = columnModels(models, 'g');
    expect(lead.map((m) => m.id)).toEqual(sidekick.map((m) => m.id));
    expect(lead.map((m) => m.id)).toEqual(['gpt-5', 'glm-4.6', 'gemini-2.5-pro']);
  });
  it('an empty query keeps the catalogue order', () => {
    expect(columnModels(models, '  ').map((m) => m.id)).toEqual(models.map((m) => m.id));
  });
});

describe('selection', () => {
  it('marks pi\'s model as single', () => {
    expect(selectionOf({ provider: 'openai', id: 'gpt-5' }, undefined)).toEqual({ kind: 'single', key: 'openai/gpt-5' });
  });
  it('marks Fusion only while pi still runs the pair\'s lead', () => {
    const pair = { kind: 'fusion' as const, lead: 'anthropic/claude-opus-4-1', sidekick: 'zai/glm-4.6' };
    expect(selectionOf({ provider: 'anthropic', id: 'claude-opus-4-1' }, pair)).toMatchObject({ kind: 'fusion' });
    expect(selectionOf({ provider: 'anthropic', id: 'claude-sonnet-4-5' }, pair)).toEqual({ kind: 'single', key: 'anthropic/claude-sonnet-4-5' });
  });
  it('returns nothing without a model', () => {
    expect(selectionOf(undefined, undefined)).toBeUndefined();
  });
});

describe('the one apply action', () => {
  it('is absent until a different row is picked', () => {
    expect(singleAction({ kind: 'single', key: 'a/b' }, undefined)).toBeUndefined();
    expect(singleAction({ kind: 'single', key: 'a/b' }, 'a/b')).toBeUndefined();
    expect(singleAction({ kind: 'single', key: 'a/b' }, 'c/d')).toEqual({ label: 'Switch model', offFusion: false });
  });
  it('turns Fusion off when a single model is picked over a pair', () => {
    expect(singleAction({ kind: 'fusion', lead: 'a/b', sidekick: 'c/d' }, 'e/f')).toEqual({ label: 'Turn Fusion off & switch', offFusion: true });
  });
  it('tapping the picked row again clears the pick', () => {
    expect(nextSinglePick({ kind: 'single', key: 'a/b' }, 'c/d', 'c/d')).toBeUndefined();
    expect(nextSinglePick({ kind: 'single', key: 'a/b' }, undefined, 'a/b')).toBeUndefined();
    expect(nextSinglePick({ kind: 'single', key: 'a/b' }, undefined, 'c/d')).toBe('c/d');
  });
  it('the running pair disables "Use this pair"', () => {
    const pair = { kind: 'fusion' as const, lead: 'a/b', sidekick: 'c/d', leadEffort: 'high', sidekickEffort: 'low' };
    expect(pairUnchanged(pair, 'a/b', 'c/d', 'high', 'low')).toBe(true);
    expect(pairUnchanged(pair, 'a/b', 'c/d', 'low', 'low')).toBe(false);
    expect(pairUnchanged({ kind: 'single', key: 'a/b' }, 'a/b', 'c/d', 'medium', 'medium')).toBe(false);
  });
});

describe('labels', () => {
  it('the Now line is sentence case, and shows the pair for Fusion', () => {
    const nameOf = (key: string) => ({ 'anthropic/claude-opus-4-1': 'Claude Opus 4.1', 'zai/glm-4.6': 'GLM-4.6', 'anthropic/claude-sonnet-4-5': 'Anthropic: Claude Sonnet 4.5' })[key] ?? key;
    expect(nowLabel({ kind: 'single', key: 'anthropic/claude-sonnet-4-5' }, nameOf)).toBe('Claude Sonnet 4.5');
    expect(nowLabel({ kind: 'fusion', lead: 'anthropic/claude-opus-4-1', sidekick: 'zai/glm-4.6' }, nameOf)).toBe('Fusion · Claude Opus 4.1 ◆ GLM-4.6');
    expect(nowLabel(undefined, nameOf)).toBe('No model yet');
  });
  it('the toast names both models (Z-72)', () => {
    expect(switchToast('Claude Sonnet 4.5', 'GPT-5')).toBe('Switched from Claude Sonnet 4.5 to GPT-5');
    expect(switchToast('GPT-5', 'GPT-5')).toBe('Switched to GPT-5');
  });
});
