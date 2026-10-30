// @vitest-environment jsdom
/**
 * Session tree sheet (UNI-158): basics — they load their list
 * when opened, show the current session/leaf marked, and call back on pick.
 * BoardSheet portals its content to document.body, so queries go through
 * `screen` (document.body), not the render container.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { ChatHeaderMenu, relativeTimeMs } from './SessionMenus';
import type { Chat, SessionsItem } from '../../lib/chat/store';

afterEach(() => (document.body.innerHTML = ''));

function fakeChat(overrides: Partial<Chat> = {}): Chat {
  const [state] = createStore({
    conn: 'live',
    run: { running: false },
    entries: [],
    truncated: false,
    tools: {},
    commands: [],
    models: [],
    dialogs: [],
    queue: [],
    pending: [],
    hasOlder: false,
    loadingOlder: false,
    seams: [],
  });
  return { state, ...overrides } as unknown as Chat;
}

describe('relativeTimeMs', () => {
  it('formats ms-since-epoch deltas', () => {
    const now = 1_000_000;
    expect(relativeTimeMs(now - 10_000, now)).toBe('just now');
    expect(relativeTimeMs(now - 90_000, now)).toBe('2m ago');
    expect(relativeTimeMs(undefined, now)).toBe('');
  });
});

describe('ChatHeaderMenu', () => {
  const anchor = { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;

  it('"New chat in this folder" (a separate pi beside this one) shows with onNewHere and runs it; "New session" stays', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('New chat in this folder')).toBeNull();
    cleanup();
    const onNewHere = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={onDismiss} onNew={() => {}} onNewHere={onNewHere} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.getByText('New session')).toBeTruthy();
    screen.getByText('New chat in this folder').click();
    expect(onDismiss).toHaveBeenCalled();
    expect(onNewHere).toHaveBeenCalled();
  });

  it('shows "Files here" only when onFilesHere is provided (a known cwd)', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Files here')).toBeNull();
  });

  it('calls onFilesHere when the row is tapped', () => {
    const onFilesHere = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onFilesHere={onFilesHere} />
    ));
    screen.getByText('Files here').click();
    expect(onFilesHere).toHaveBeenCalled();
  });

  it('UNI-217: "Git" shows only with onGit (a known cwd) and runs it', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Git')).toBeNull();
    cleanup();
    const onGit = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={onDismiss} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onGit={onGit} />
    ));
    screen.getByText('Git').click();
    expect(onDismiss).toHaveBeenCalled();
    expect(onGit).toHaveBeenCalled();
  });

  it('UNI-188: shows Running / Context / Session info only with onOpenSection, with the running count hint', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Running')).toBeNull();

    cleanup();
    const onOpenSection = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu
        anchor={anchor}
        onDismiss={onDismiss}
        onNew={() => {}}
       
        onTree={() => {}}
        onRename={() => {}}
        canRename
        onOpenSection={onOpenSection}
        runningCount={2}
      />
    ));
    expect(screen.getByText('Running').parentElement?.textContent).toContain('2');
    screen.getByText('Running').click();
    expect(onOpenSection).toHaveBeenCalledWith('running');
    expect(onDismiss).toHaveBeenCalled();
    screen.getByText('Context').click();
    expect(onOpenSection).toHaveBeenCalledWith('context');
    screen.getByText('Session info').click();
    expect(onOpenSection).toHaveBeenCalledWith('info');
  });
});
