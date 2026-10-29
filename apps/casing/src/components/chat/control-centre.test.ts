import { describe, expect, it } from 'vitest';

import { shortModelName } from './ControlCentre';
describe('shortModelName', () => {
  it('drops the provider prefix only', () => {
    expect(shortModelName('Anthropic: Claude Fable 5')).toBe('Claude Fable 5');
    expect(shortModelName('DeepSeek: DeepSeek Flash Lite')).toBe('DeepSeek Flash Lite');
    expect(shortModelName('gpt-5')).toBe('gpt-5');
    expect(shortModelName(undefined)).toBe('');
  });
});

import { pickerModels } from './ControlCentre';
describe('pickerModels (Single list = current + curated, search spans everything)', () => {
  const all = [
    ...Array.from({ length: 50 }, (_, i) => ({ provider: 'openrouter', id: `x/m${i}` })),
    { provider: 'gateway', id: 'cc/claude-fable-5-1', name: 'Claude Fable 5.1' },
    { provider: 'gateway', id: 'zai/glm-5.3' },
    { provider: 'antigravity', id: 'claude-opus-5-5' },
  ];
  const curated = [{ key: 'gateway/zai/glm-5.3' }, { key: 'antigravity/claude-opus-5-5' }, { key: 'gone/model' }];
  it('shows only current + curated (no openrouter flood)', () => {
    const list = pickerModels(all, curated, { provider: 'gateway', id: 'cc/claude-fable-5-1' }, '');
    expect(list.map((m) => `${m.provider}/${m.id}`)).toEqual(['gateway/cc/claude-fable-5-1', 'gateway/zai/glm-5.3', 'antigravity/claude-opus-5-5']);
  });
  it('search covers the whole catalogue, curated first', () => {
    const list = pickerModels(all, curated, undefined, 'm1');
    expect(list.length).toBeGreaterThan(5);
    expect(pickerModels(all, curated, undefined, 'opus')[0]!.id).toBe('claude-opus-5-5');
  });
  it('nothing curated → the bridge-ordered head', () => {
    expect(pickerModels(all, [], undefined, '').length).toBe(40);
  });
});
