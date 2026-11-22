// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { clearDraft, loadDraft, saveDraftChoices, saveDraftCwd } from './draftStored';

beforeEach(() => localStorage.clear());

describe('draftStored', () => {
  it('round-trips choices and cwd', () => {
    const choices = {
      model: { provider: 'anthropic', modelId: 'claude-sonnet-5-5', name: 'Claude Sonnet 5.5' },
      thinking: 'high', permission: 'ask', autonomy: 'medium' as const, plan: true,
      fusion: { lead: 'anthropic/claude-sonnet-5-5', sidekick: 'zai/glm-5-3', leadEffort: 'high', sidekickEffort: 'low' },
    };
    saveDraftChoices('h', choices);
    saveDraftCwd('h', '/work/project');
    expect(loadDraft('h')).toEqual({ choices, cwd: '/work/project' });
  });

  it('tolerates malformed JSON and wrong shapes', () => {
    localStorage.setItem('unipi:draft:choices:h', '{');
    expect(loadDraft('h')).toEqual({});
    localStorage.setItem('unipi:draft:choices:h', JSON.stringify({ choices: { model: { provider: 'x' }, fusion: [], plan: 'yes' }, cwd: 42 }));
    expect(loadDraft('h')).toEqual({});
  });

  it('clears cwd without dropping choices', () => {
    saveDraftChoices('h', { thinking: 'high' });
    saveDraftCwd('h', '/work');
    saveDraftCwd('h', undefined);
    expect(loadDraft('h')).toEqual({ choices: { thinking: 'high' } });
  });

  it('clears the complete record', () => {
    saveDraftChoices('h', { plan: true });
    saveDraftCwd('h', '/work');
    clearDraft('h');
    expect(loadDraft('h')).toEqual({});
  });

  it('stores records per host', () => {
    saveDraftChoices('h1', { thinking: 'low' });
    saveDraftCwd('h2', '/other');
    expect(loadDraft('h1')).toEqual({ choices: { thinking: 'low' } });
    expect(loadDraft('h2')).toEqual({ cwd: '/other' });
  });
});
