import { describe, expect, it } from 'vitest';
import { findModel } from './find';

const models = [
  { provider: 'oino', id: 'ollamacloud/deepseek-v4.1-flash' },
  { provider: 'oino', id: 'ollamacloud/glm-4.7' },
  { provider: 'anthropic', id: 'claude-opus-4-5', name: 'Claude Opus 4.5' },
  { provider: 'anthropic', id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
];

describe('findModel (/model <name>)', () => {
  it('matches keys, ids, display names and a unique substring', () => {
    expect(findModel(models, 'oino/ollamacloud/glm-4.7')?.id).toBe('ollamacloud/glm-4.7');
    expect(findModel(models, 'claude-opus-4-5')?.id).toBe('claude-opus-4-5');
    expect(findModel(models, 'DeepSeek V4.1 Flash')?.id).toBe('ollamacloud/deepseek-v4.1-flash');
    expect(findModel(models, 'sonnet')?.id).toBe('claude-sonnet-4-5');
  });
  it('an ambiguous or unknown name is no match', () => {
    expect(findModel(models, 'claude')).toBeUndefined();
    expect(findModel(models, 'gpt')).toBeUndefined();
  });
});
