// @vitest-environment jsdom
/** Deep links (G-12, WP-A19): `unipi://s/<host>/<session>` routes to the
 * session; the v1 `unipi://chat?host=&session=` form still opens it; anything
 * without a session lands on the Chat list. */
import { afterEach, describe, expect, it } from 'vitest';
import { openChatFromDeepLink, openSessionRoute, resolveHostId } from './notifyInit';
import type { HostInfo } from './schemas';

const host: HostInfo = { id: 'host-1', name: 'oi', ticket: 't', paired_at: 1, last_path: 'unknown' };

afterEach(() => {
  location.hash = '';
});

describe('openSessionRoute', () => {
  it('sets the conversation route hash (ids are URL-encoded)', () => {
    openSessionRoute('host-1', 's-4242');
    expect(location.hash).toBe('#/s/host-1/s-4242');
    openSessionRoute('my pc', 'abc/def');
    expect(location.hash).toBe(`#/s/${encodeURIComponent('my pc')}/${encodeURIComponent('abc/def')}`);
  });
});

describe('openChatFromDeepLink', () => {
  it('a v2 path link opens the session route', () => {
    openChatFromDeepLink([host], 'unipi://s/host-1/s-4242');
    expect(location.hash).toBe('#/s/host-1/s-4242');
  });

  it('host names resolve like ids; ids are decoded', () => {
    openChatFromDeepLink([host], `unipi://s/${encodeURIComponent('oi')}/s-1`);
    expect(location.hash).toBe('#/s/host-1/s-1');
  });

  it('the v1 query form still opens the session', () => {
    openChatFromDeepLink([host], 'unipi://chat?host=host-1&session=s99');
    expect(location.hash).toBe('#/s/host-1/s99');
  });

  it('links without a session (v1 pid, unknown host) go to the Chat list', () => {
    openChatFromDeepLink([host], 'unipi://chat?host=host-1&pid=4242');
    expect(location.hash).toBe('#/chat');
    openChatFromDeepLink([], 'unipi://chat?host=x&session=s1');
    expect(location.hash).toBe('#/chat');
  });

  it('ignores other links', () => {
    location.hash = '#/board';
    openChatFromDeepLink([host], 'unipi://pair?t=x');
    expect(location.hash).toBe('#/board');
  });
});

describe('resolveHostId', () => {
  it('id, then name (any case), then the first host', () => {
    expect(resolveHostId([host], 'host-1')).toBe('host-1');
    expect(resolveHostId([host], 'OI')).toBe('host-1');
    expect(resolveHostId([host], undefined)).toBe('host-1');
  });
});
