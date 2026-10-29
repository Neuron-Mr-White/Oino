import { describe, expect, it } from 'vitest';
import { friendlyBridgeError, isUnknownTypeError } from './bridgeError';

describe('UNI-308: friendlyBridgeError', () => {
  it('maps the bridge wire error "unknown type <t>" (an older pi than the app) to a friendly message', () => {
    expect(friendlyBridgeError('unknown type session_new')).toBe('This pi is older than the app — restart that pi session to use this.');
    expect(friendlyBridgeError('Unknown type reload')).toBe('This pi is older than the app — restart that pi session to use this.');
    expect(friendlyBridgeError('  unknown type foo  ')).toBe('This pi is older than the app — restart that pi session to use this.');
  });

  it('passes through a real failure unchanged', () => {
    expect(friendlyBridgeError('pi is busy. Stop the current run first.')).toBe('pi is busy. Stop the current run first.');
    expect(friendlyBridgeError("That file doesn't exist on the host.")).toBe("That file doesn't exist on the host.");
  });

  it('falls back to a generic message for missing/empty text', () => {
    expect(friendlyBridgeError(undefined)).toBe('Something went wrong');
    expect(friendlyBridgeError(null)).toBe('Something went wrong');
    expect(friendlyBridgeError('')).toBe('Something went wrong');
  });

  it('isUnknownTypeError matches only the bridge wire shape', () => {
    expect(isUnknownTypeError('unknown type usage')).toBe(true);
    expect(isUnknownTypeError('unknowntype usage')).toBe(false);
    expect(isUnknownTypeError('something about unknown type elsewhere')).toBe(false);
    expect(isUnknownTypeError(undefined)).toBe(false);
  });
});
