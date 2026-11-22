import { describe, expect, it } from 'vitest';
import { fuzzyMatch, fuzzyRank } from './fuzzy';

const models = ['Claude Opus 5.5', 'Claude Sonnet 5.5', 'GLM 5.3', 'GLM 5.3 Flash', 'Gemini 4 Argon (High)', 'DeepSeek V4.1 Flash', 'GPT 6.1 Sol'];
const rank = (query: string) => fuzzyRank(query, models, (name) => [name]).map((row) => row.item);

describe('fuzzy matcher', () => {
  it('requires every character and handles blank queries', () => {
    expect(fuzzyMatch('z', 'Flash')).toBeNull();
    expect(fuzzyMatch('  ', 'Flash')).toEqual({ score: 0, positions: [] });
  });
  it('matches terms in any order and returns positions', () => {
    expect(fuzzyMatch('flash deep', 'DeepSeek V4 Flash')?.positions).toEqual([0, 1, 2, 3, 12, 13, 14, 15, 16]);
  });
  it('ranks model shorthand and exact substrings', () => {
    expect(rank('son')[0]).toBe('Claude Sonnet 5.5');
    expect(rank('glmf')[0]).toBe('GLM 5.3 Flash');
    expect(rank('gem4')[0]).toBe('Gemini 4 Argon (High)');
    expect(rank('opus')[0]).toBe('Claude Opus 5.5');
    expect(rank('g53')[0]).toBe('GLM 5.3');
    expect(fuzzyMatch('son55', 'Claude Sonnet 5.5')).not.toBeNull();
    expect(fuzzyMatch('flash', 'GLM 5.3 Flash')!.score).toBeGreaterThan(fuzzyMatch('flash', 'f---l---a---s---h')!.score);
  });
  it('keeps ties stable and matches free-text term order', () => {
    expect(fuzzyMatch('flash deep', 'DeepSeek V4 Flash')).not.toBeNull();
    expect(fuzzyRank('a', ['a', 'a'], (x) => [x]).map((r) => r.item)).toEqual(['a', 'a']);
  });
});
