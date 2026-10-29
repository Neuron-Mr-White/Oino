import { describe, it, expect, afterEach } from 'vitest';
import { parseChatDeepLink, readPendingAction } from './notifyDeepLink';

describe('parseChatDeepLink', () => {
  it('parses host/pid/dialog', () => {
    expect(parseChatDeepLink('unipi://chat?host=pc1&pid=42&dialog=3')).toEqual({ host: 'pc1', pid: 42, dialog: 3 });
  });

  it('parses a bare unipi://chat (no params)', () => {
    expect(parseChatDeepLink('unipi://chat')).toEqual({});
  });

  it('returns undefined for unipi://pair or any other scheme', () => {
    expect(parseChatDeepLink('unipi://pair?t=x&c=y')).toBeUndefined();
    expect(parseChatDeepLink('https://example.com')).toBeUndefined();
    expect(parseChatDeepLink('not a url')).toBeUndefined();
  });
});

describe('readPendingAction', () => {
  afterEach(() => {
    delete (window as unknown as { UniPiNotify?: unknown }).UniPiNotify;
  });

  it('returns undefined when the native bridge is absent', () => {
    expect(readPendingAction()).toBeUndefined();
  });

  it('parses a valid pending action from the bridge', () => {
    (window as unknown as { UniPiNotify: { takePendingAction: () => string } }).UniPiNotify = {
      takePendingAction: () => JSON.stringify({ notificationId: 7, action: 'Yes' }),
    };
    expect(readPendingAction()).toEqual({ notificationId: 7, action: 'Yes' });
  });

  it('returns undefined when the bridge has nothing queued', () => {
    (window as unknown as { UniPiNotify: { takePendingAction: () => string | null } }).UniPiNotify = {
      takePendingAction: () => null,
    };
    expect(readPendingAction()).toBeUndefined();
  });

  it('returns undefined for malformed JSON from the bridge', () => {
    (window as unknown as { UniPiNotify: { takePendingAction: () => string } }).UniPiNotify = {
      takePendingAction: () => 'not json',
    };
    expect(readPendingAction()).toBeUndefined();
  });
});
