// @vitest-environment jsdom
/**
 * UNI-257: the chat ⋯ menu's Git row shows the repo's total "+add −del"
 * (green/red), refreshed from the host each time the menu opens; nothing
 * when clean, outside a repo, or on an older host (E_SCHEMA).
 */
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const call = vi.fn();
vi.mock('../../lib/ipc', () => ({ call: (...a: unknown[]) => call(...a) }));

import { ChatHeaderMenu } from './SessionMenus';
import { compactCount, menuStatOf } from '../../lib/git';

const anchor = { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
let n = 0;
const menu = (path = `/p/${++n}`) =>
  render(() => (
    <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onGit={() => {}} gitStat={{ hostId: 'h1', path }} />
  ));

beforeEach(() => call.mockReset());
afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

describe('Git row +/- (UNI-257)', () => {
  it('asks the host for a shortstat of the chat cwd and shows +add −del', async () => {
    call.mockResolvedValue({ ok: true, value: { repo: true, files: 3, add: 120, del: 34, capped: false } });
    menu('/home/u/proj');
    await waitFor(() => expect(screen.getByTestId('chat-menu-git-stat')).toBeTruthy());
    expect(call).toHaveBeenCalledWith('git_op', expect.objectContaining({ hostId: 'h1', path: '/home/u/proj', cmd: { action: 'shortstat' } }), expect.anything());
    const stat = screen.getByTestId('chat-menu-git-stat');
    expect(stat.querySelector('.add')?.textContent).toBe('+120');
    expect(stat.querySelector('.del')?.textContent).toBe('−34');
    expect(screen.getByText('Git')).toBeTruthy();
  });

  it('shows nothing when the tree is clean', async () => {
    call.mockResolvedValue({ ok: true, value: { repo: true, files: 0, add: 0, del: 0, capped: false } });
    menu();
    await waitFor(() => expect(call).toHaveBeenCalled());
    await Promise.resolve();
    expect(screen.queryByTestId('chat-menu-git-stat')).toBeNull();
    expect(screen.getByText('Git')).toBeTruthy();
  });

  it('shows nothing outside a repo or on an older host', async () => {
    call.mockResolvedValue({ ok: true, value: { repo: false, files: 0, add: 0, del: 0, capped: false } });
    menu();
    await waitFor(() => expect(call).toHaveBeenCalled());
    expect(screen.queryByTestId('chat-menu-git-stat')).toBeNull();
    cleanup();
    call.mockResolvedValue({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `shortstat`' } });
    menu();
    await waitFor(() => expect(call).toHaveBeenCalledTimes(2));
    expect(screen.queryByTestId('chat-menu-git-stat')).toBeNull();
  });

  it('refreshes on every open and shows the last value at once', async () => {
    const path = '/p/refresh';
    call.mockResolvedValue({ ok: true, value: { repo: true, files: 1, add: 5, del: 1, capped: false } });
    menu(path);
    await waitFor(() => expect(screen.getByTestId('chat-menu-git-stat').textContent).toContain('+5'));
    cleanup();
    let resolve!: (v: unknown) => void;
    call.mockReturnValue(new Promise((r) => (resolve = r)));
    menu(path);
    // Cached value before the host answers…
    expect(screen.getByTestId('chat-menu-git-stat').textContent).toContain('+5');
    resolve({ ok: true, value: { repo: true, files: 2, add: 9, del: 0, capped: true } });
    // …then the fresh one (capped untracked → "+9+").
    await waitFor(() => expect(screen.getByTestId('chat-menu-git-stat').querySelector('.add')?.textContent).toBe('+9+'));
    expect(call).toHaveBeenCalledTimes(2);
  });

  it('no gitStat prop → no request', () => {
    render(() => <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onTree={() => {}} onRename={() => {}} canRename onGit={() => {}} />);
    expect(call).not.toHaveBeenCalled();
  });
});

describe('menuStatOf / compactCount', () => {
  it('maps replies', () => {
    expect(menuStatOf({ ok: true, value: { repo: true, add: 1, del: 0, capped: false } })).toEqual({ add: 1, del: 0, capped: false });
    expect(menuStatOf({ ok: true, value: { repo: true, add: 0, del: 0, capped: false } })).toBeNull();
    expect(menuStatOf({ ok: true, value: { repo: false, add: 3, del: 0, capped: false } })).toBeNull();
    expect(menuStatOf({ ok: false, error: { code: 'E_SCHEMA', message: 'x' } } as never)).toBeNull();
  });
  it('compacts big counts', () => {
    expect(compactCount(999)).toBe('999');
    expect(compactCount(9999)).toBe('9999');
    expect(compactCount(12345)).toBe('12.3k');
    expect(compactCount(250000)).toBe('250k');
    expect(compactCount(1_500_000)).toBe('1.5M');
  });
});
