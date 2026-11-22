import { describe, expect, it } from 'vitest';
import { cleanReason, errorCopy } from './errorCopy';

describe('errorCopy (phone spec §3.4, PH-14)', () => {
  it('maps the setup errors to a fix, never CLI wording or host paths', () => {
    const c = errorCopy('No model selected. Use /login to log into a provider via OAuth or API key. See: /root/qa/agent/docs/docs/providers.md');
    expect(c).toEqual({ title: 'No model selected', reason: 'Choose a model for this chat, then retry.', fix: 'model' });
    expect(errorCopy('Anthropic rejected the key (401) — fix it in Settings › Providers.').fix).toBe('provider');
    expect(errorCopy('OpenRouter rate-limited the request (429). Try again shortly.').fix).toBeUndefined();
  });
  it('falls back to the first line without paths or /commands', () => {
    const c = errorCopy('Something odd happened in /home/me/.oino/agent/x.json. Use /reload to retry.');
    expect(c.title).toBe('The model request failed');
    expect(c.reason).not.toMatch(/\/home|\/reload/);
    expect(cleanReason('Boom at /root/a/b.txt')).toBe('Boom at');
  });
});
