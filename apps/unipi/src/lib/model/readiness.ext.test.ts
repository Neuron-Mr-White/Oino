import { describe, expect, it } from 'vitest';
import { readinessOf, withModelProviders } from './readiness';

const builtins = readinessOf([
  { id: 'openrouter', name: 'OpenRouter', oauth: false, configured: true, env: [], ready: false },
  { id: 'anthropic', name: 'Anthropic', oauth: false, configured: false, env: [] },
]);

describe('extension-registered providers count as usable', () => {
  it('a router-only PC is not "no provider"', () => {
    expect(builtins.any).toBe(false);
    expect(withModelProviders(builtins, ['oino', 'openrouter'])?.any).toBe(true);
  });
  it('only built-in providers in the list keep the honest answer', () => {
    expect(withModelProviders(builtins, ['openrouter', 'anthropic', undefined])?.any).toBe(false);
  });
  it('a provider the host does not list is ready; a rejected one is not', () => {
    expect(builtins.providerReady('oino')).toBe(true);
    expect(builtins.providerReady('openrouter')).toBe(false);
  });
});
