// @vitest-environment jsdom
/**
 * WP-A25 helpers around the Providers page (G-15 `/login` `/logout`): the
 * state line each provider shows, and the OAuth URL the app opens in the
 * system browser when a provider's sign-in dialog carries one.
 */
import { describe, expect, it } from 'vitest';
import { oauthUrlIn, providerState, type Provider } from '../settings/ProvidersPage';

const provider = (patch: Partial<Provider> = {}): Provider => ({ id: 'anthropic', name: 'Anthropic', oauth: false, configured: false, env: [], ...patch });

describe('providerState', () => {
  it('reads the stored credential, then the environment', () => {
    expect(providerState(provider({ stored: 'oauth', configured: true }))).toEqual({ label: 'Signed in', detail: 'OAuth' });
    expect(providerState(provider({ stored: 'api_key', configured: true }))).toEqual({ label: 'API key saved' });
    expect(providerState(provider({ configured: true, env: ['ANTHROPIC_API_KEY'] }))).toEqual({ label: 'Ready', detail: 'from ANTHROPIC_API_KEY' });
    expect(providerState(provider())).toEqual({ label: 'Not configured' });
  });
});

describe('oauthUrlIn', () => {
  it('finds the URL a provider asks the user to open', () => {
    expect(oauthUrlIn({ method: 'input', message: 'Open https://auth.example.com/authorize?x=1 and paste the code' })).toBe('https://auth.example.com/authorize?x=1');
    expect(oauthUrlIn({ method: 'input', placeholder: 'https://claude.ai/oauth/authorize' })).toBe('https://claude.ai/oauth/authorize');
    expect(oauthUrlIn({ url: 'http://localhost:1455/callback' })).toBe('http://localhost:1455/callback');
  });

  it('is undefined when the dialog has no URL (no external open)', () => {
    expect(oauthUrlIn({ method: 'confirm', title: 'Sign in with your browser?' })).toBeUndefined();
    expect(oauthUrlIn({ method: 'input', message: 'Paste the code', placeholder: 'code' })).toBeUndefined();
  });

  it('ignores a non-http scheme', () => {
    expect(oauthUrlIn({ message: 'file:///etc/passwd' })).toBeUndefined();
  });
});
