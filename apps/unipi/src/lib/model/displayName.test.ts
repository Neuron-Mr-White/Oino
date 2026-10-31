import { describe, expect, it } from 'vitest';
import { modelDisplayName, modelOrigin, prettyModelId } from './displayName';

describe('model display names', () => {
  it('derives a readable name from ids', () => {
    expect(prettyModelId('ollamacloud/deepseek-v4.1-flash')).toBe('DeepSeek V4.1 Flash');
    expect(prettyModelId('ollamacloud/glm-4.7')).toBe('GLM 4.7');
    expect(prettyModelId('anthropic/claude-opus-4-5')).toBe('Claude Opus 4.5');
    expect(prettyModelId('gpt-5.1-codex-mini')).toBe('GPT 5.1 Codex Mini');
    expect(prettyModelId('qwen3-coder:30b')).toBe('Qwen3 Coder 30B');
    expect(prettyModelId('o3-mini')).toBe('o3 Mini');
  });

  it('keeps a real catalogue name, prettifies one that is an id', () => {
    expect(modelDisplayName({ id: 'claude-x', name: 'Claude Fable 5' })).toBe('Claude Fable 5');
    expect(modelDisplayName({ id: 'x', name: 'Anthropic: Claude Fable 5' })).toBe('Claude Fable 5');
    expect(modelDisplayName({ id: 'ollamacloud/deepseek-v4.1-flash', name: 'ollamacloud/deepseek-v4.1-flash' })).toBe('DeepSeek V4.1 Flash');
    expect(modelDisplayName({ id: 'deepseek-flash' })).toBe('DeepSeek Flash');
    expect(modelDisplayName(undefined)).toBe('');
  });

  it('names the origin for tooltips', () => {
    expect(modelOrigin({ provider: 'oino', id: 'ollamacloud/deepseek-v4.1-flash' })).toBe('oino · ollamacloud/deepseek-v4.1-flash');
  });
});
