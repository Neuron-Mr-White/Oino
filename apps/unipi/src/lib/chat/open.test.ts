import { describe, expect, it } from 'vitest';
import { chatVisibilityOnScreenChange } from './open';

describe('chatVisibilityOnScreenChange (UNI-312)', () => {
  it('leaves chatVisible alone on mount (no previous screen)', () => {
    expect(chatVisibilityOnScreenChange(undefined, 'chat', true)).toBeUndefined();
    expect(chatVisibilityOnScreenChange(undefined, 'board', true)).toBeUndefined();
  });

  it('leaves chatVisible alone when the screen does not actually change', () => {
    expect(chatVisibilityOnScreenChange('chat', 'chat', true)).toBeUndefined();
    expect(chatVisibilityOnScreenChange('board', 'board', false)).toBeUndefined();
  });

  it('hides the chat layer when navigating away from chat to another screen', () => {
    expect(chatVisibilityOnScreenChange('chat', 'board', true)).toBe(false);
    expect(chatVisibilityOnScreenChange('chat', 'files', true)).toBe(false);
    expect(chatVisibilityOnScreenChange('chat', 'sessions', true)).toBe(false);
  });

  it('hides the chat layer leaving chat even with none open (idempotent)', () => {
    expect(chatVisibilityOnScreenChange('chat', 'board', false)).toBe(false);
  });

  it('shows the open chat again when navigating back to chat', () => {
    expect(chatVisibilityOnScreenChange('board', 'chat', true)).toBe(true);
    expect(chatVisibilityOnScreenChange('files', 'chat', true)).toBe(true);
  });

  it('leaves chatVisible alone navigating to chat with none open (the list renders as usual)', () => {
    expect(chatVisibilityOnScreenChange('board', 'chat', false)).toBeUndefined();
  });

  it('leaves chatVisible alone for non-chat-to-non-chat transitions', () => {
    expect(chatVisibilityOnScreenChange('board', 'files', true)).toBeUndefined();
    expect(chatVisibilityOnScreenChange('sessions', 'settings', false)).toBeUndefined();
  });
});
