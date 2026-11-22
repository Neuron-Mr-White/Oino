// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import {
  activeChatTargetFor,
  chatVisibilityOnScreenChange,
  clearReturnToChat,
  hasMarkedReturnToChat,
  markReturnToChat,
  openChat,
  openNewChat,
  openTaskFromChat,
  resumeChatAfterTerminal,
} from './open';

afterEach(() => {
  clearReturnToChat();
  location.hash = '#/';
});

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

describe('v2 chat routing & return-route (P0-07)', () => {
  it('openChat navigates to #/s/<host>/<session> and openNewChat to #/new/<host>?cwd=', () => {
    openChat({ hostId: 'local', session: 'sess-9' });
    expect(location.hash).toBe('#/s/local/sess-9');
    expect(activeChatTargetFor('local')).toMatchObject({
      kind: 'session',
      hostId: 'local',
      sessionId: 'sess-9',
      mentionKey: 'local:sess-9',
      route: '/s/local/sess-9',
    });

    openNewChat('local', '/home/u/proj');
    expect(location.hash).toBe('#/new/local?cwd=%2Fhome%2Fu%2Fproj');
    expect(activeChatTargetFor('local')).toMatchObject({
      kind: 'draft',
      hostId: 'local',
      cwd: '/home/u/proj',
      mentionKey: 'draft:local',
    });
  });

  it('returns to the exact v2 chat route when closing Board or Files', () => {
    location.hash = '#/s/local/s42';
    openTaskFromChat('local', 'UNI-10', '/home/u/proj');
    expect(location.hash).toBe('#/board');
    expect(hasMarkedReturnToChat()).toBe(true);
    expect(activeChatTargetFor('local')).toMatchObject({ kind: 'session', sessionId: 's42' });

    expect(resumeChatAfterTerminal()).toBe(true);
    expect(location.hash).toBe('#/s/local/s42');
    expect(hasMarkedReturnToChat()).toBe(false);

    markReturnToChat('/new/local?cwd=%2Ftmp');
    location.hash = '#/files';
    expect(resumeChatAfterTerminal()).toBe(true);
    expect(location.hash).toBe('#/new/local?cwd=%2Ftmp');
  });
});
