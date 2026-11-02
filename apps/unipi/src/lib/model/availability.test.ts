import { describe, expect, it } from 'vitest';
import { calmModelError, isChatModel, splitAvailability } from './availability';
import { readinessOf } from './readiness';
import type { ModelInfo } from './picker';

const m = (provider: string, id: string, extra: Partial<ModelInfo> = {}): ModelInfo => ({ provider, id, ...extra });

describe('F-02 availability', () => {
  it('hides non-chat models', () => {
    expect(isChatModel(m('mm', 'speech-2.8-hd', { name: 'Speech 2.8 Hd' }))).toBe(false);
    expect(isChatModel(m('mm', 'image-01'))).toBe(false);
    expect(isChatModel(m('x', 'text-embedding-3-small'))).toBe(false);
    expect(isChatModel(m('x', 'claude-haiku-4.5'))).toBe(true);
    expect(isChatModel(m('x', 'a', { kind: 'image' }))).toBe(false);
    expect(isChatModel(m('x', 'a', { kind: 'chat' }))).toBe(true);
  });
  it('sets models of a rejected or missing provider apart, with the reason; keeps the running one', () => {
    const readiness = readinessOf([
      { id: 'oino', name: 'Oino', oauth: false, configured: true, env: [], ready: true },
      { id: 'openrouter', name: 'OpenRouter', oauth: false, configured: true, env: [], ready: false, reason: 'the key was rejected (401)' },
      { id: 'zai', name: 'Z.ai', oauth: false, configured: false, env: [] },
    ]);
    const all = [m('oino', 'deepseek'), m('openrouter', 'haiku'), m('zai', 'glm'), m('oino', 'image-01')];
    const r = splitAvailability(all, readiness);
    expect(r.usable.map((x) => x.id)).toEqual(['deepseek']);
    expect(r.unavailable.map((u) => u.reason)).toEqual(['OpenRouter: the key was rejected (401)', 'Z.ai isn’t set up']);
    expect(splitAvailability(all, readiness, 'openrouter/haiku').usable.map((x) => x.id)).toEqual(['deepseek', 'haiku']);
  });
  it('trusts the host’s per-model readiness', () => {
    const r = splitAvailability([m('a', 'b', { ready: false, reason: 'no quota' })], undefined);
    expect(r.unavailable[0]?.reason).toBe('no quota');
  });
  it('never shows provider JSON', () => {
    const text = calmModelError(new Error('401 {"type":"error","error":{"type":"authentication_error"}}'), 'Haiku');
    expect(text).not.toContain('{');
    expect(text).toContain('rejected the key');
  });
});
