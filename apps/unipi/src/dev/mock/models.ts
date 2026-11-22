/**
 * Mock model catalogue (WP-A14): what the mock host answers for
 * `get_available_models` / `get_available_thinking_levels`, and the models
 * `set_model` / `cycle_thinking_level` switch between. Shaped like pi's
 * `Model` JSON (camelCase, `input` modalities, `contextWindow`, `reasoning`)
 * so the picker's parsing, the Vision/context badges and the thinking chip
 * are exercised for real in captures and component tests.
 */

/** One catalogue row (the fields the picker reads). */
export type MockModel = {
  provider: string;
  id: string;
  name: string;
  reasoning: boolean;
  input: string[];
  contextWindow: number;
  maxTokens: number;
};

export const MOCK_MODELS: MockModel[] = [
  { provider: 'anthropic', id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5', reasoning: true, input: ['text', 'image'], contextWindow: 200_000, maxTokens: 64_000 },
  { provider: 'anthropic', id: 'claude-opus-4-1', name: 'Claude Opus 4.1', reasoning: true, input: ['text', 'image'], contextWindow: 200_000, maxTokens: 32_000 },
  { provider: 'anthropic', id: 'claude-haiku-4', name: 'Claude Haiku 4', reasoning: true, input: ['text', 'image'], contextWindow: 200_000, maxTokens: 32_000 },
  { provider: 'openai', id: 'gpt-5', name: 'GPT-5', reasoning: true, input: ['text', 'image'], contextWindow: 400_000, maxTokens: 128_000 },
  { provider: 'openai', id: 'gpt-5-mini', name: 'GPT-5 mini', reasoning: true, input: ['text', 'image'], contextWindow: 400_000, maxTokens: 128_000 },
  { provider: 'openai', id: 'o3', name: 'o3', reasoning: true, input: ['text'], contextWindow: 200_000, maxTokens: 100_000 },
  { provider: 'google', id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', reasoning: true, input: ['text', 'image'], contextWindow: 1_000_000, maxTokens: 65_536 },
  { provider: 'google', id: 'gemini-2.5-flash', name: 'Gemini 2.5 Flash', reasoning: true, input: ['text', 'image'], contextWindow: 1_000_000, maxTokens: 65_536 },
  { provider: 'zai', id: 'glm-4.6', name: 'GLM-4.6', reasoning: true, input: ['text'], contextWindow: 200_000, maxTokens: 128_000 },
  { provider: 'zai', id: 'glm-4.5-air', name: 'GLM-4.5 Air', reasoning: false, input: ['text'], contextWindow: 128_000, maxTokens: 32_000 },
  { provider: 'deepseek', id: 'deepseek-reasoner', name: 'DeepSeek Reasoner', reasoning: true, input: ['text'], contextWindow: 128_000, maxTokens: 64_000 },
  { provider: 'meta', id: 'llama-4-maverick', name: 'Llama 4 Maverick', reasoning: false, input: ['text', 'image'], contextWindow: 128_000, maxTokens: 16_384 },
];

/** The levels a reasoning mock model supports (pi's base five). */
export const MOCK_THINKING_LEVELS = ['off', 'minimal', 'low', 'medium', 'high'];

/** The catalogue row for a `provider`/`id` pair. */
export function mockModelFor(provider: string, id: string): MockModel | undefined {
  return MOCK_MODELS.find((m) => m.provider === provider && m.id === id);
}

/** What `get_available_thinking_levels` answers for a model. */
export function mockLevelsFor(model: { reasoning?: boolean } | undefined): string[] {
  return model && model.reasoning === false ? ['off'] : MOCK_THINKING_LEVELS;
}
