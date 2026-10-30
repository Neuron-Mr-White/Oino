import { describe, expect, it } from 'vitest';
import { allCommands, paneFor, rankCommands, registerCommands, registerPane, screenOf, type Command } from './shell';
import { clampListWidth, DEFAULT_LIST_WIDTH, defaultListWidth, MIN_DETAIL, MIN_LIST, wantsDesktopShell } from './layout';

const cmd = (id: string, title: string, group = 'G', keywords?: string): Command => ({ id, title, group, keywords, run: () => {} });

describe('shell registries', () => {
  it('screenOf maps routes to screen keys', () => {
    expect(screenOf('/')).toBe('chat');
    expect(screenOf('/files/edit')).toBe('files');
    expect(screenOf('/board')).toBe('board');
  });

  it('registerPane / unregister', () => {
    const off = registerPane({ screen: 'x', open: () => true });
    expect(paneFor('x')?.open()).toBe(true);
    off();
    expect(paneFor('x')).toBeUndefined();
  });

  it('a stale unregister does not remove a newer spec for the same screen', () => {
    const offOld = registerPane({ screen: 'y', open: () => false });
    const newer = { screen: 'y', open: () => true };
    const offNew = registerPane(newer);
    offOld();
    expect(paneFor('y')).toBe(newer);
    offNew();
  });

  it('registerCommands replaces by id, removes on cleanup, honours when()', () => {
    const offA = registerCommands([cmd('t.a', 'Alpha'), { ...cmd('t.hidden', 'Hidden'), when: () => false }]);
    const offB = registerCommands([cmd('t.a', 'Alpha 2')]);
    const titles = allCommands().map((c) => c.title);
    expect(titles).toContain('Alpha 2');
    expect(titles).not.toContain('Alpha');
    expect(titles).not.toContain('Hidden');
    offB();
    offA();
    expect(allCommands().some((c) => c.id.startsWith('t.'))).toBe(false);
  });

  it('rankCommands: prefix > word start > substring > keyword > subsequence; all words must match', () => {
    const list = [cmd('1', 'Go to Files', 'Go to'), cmd('2', 'Filter tasks', 'Board'), cmd('3', 'Display options', 'Board', 'lanes columns'), cmd('4', 'New task', 'Board')];
    expect(rankCommands(list, 'fil').map((c) => c.id)).toEqual(['2', '1']);
    expect(rankCommands(list, 'columns').map((c) => c.id)).toEqual(['3']);
    expect(rankCommands(list, 'board task').map((c) => c.id)).toEqual(['2', '4']); // tie → registration order
    expect(rankCommands(list, 'nwtsk').map((c) => c.id)).toEqual(['4']);
    expect(rankCommands(list, 'zzz')).toEqual([]);
    expect(rankCommands(list, '  ')).toEqual(list);
  });
});

describe('layout', () => {
  it('desktop shell only at ≥ 900px and never in the phone app', () => {
    expect(wantsDesktopShell(1440, 'web', true)).toBe(true);
    expect(wantsDesktopShell(1440, 'tauri-desktop', true)).toBe(true);
    expect(wantsDesktopShell(899, 'tauri-desktop', true)).toBe(false);
    expect(wantsDesktopShell(915, 'tauri-mobile', true)).toBe(false); // phone in landscape
    expect(wantsDesktopShell(1440, 'web', false)).toBe(false); // no matchMedia (tests)
  });

  it('list width keeps both panes usable', () => {
    expect(clampListWidth(100, 1200)).toBe(MIN_LIST);
    expect(clampListWidth(5000, 1200)).toBe(1200 - MIN_DETAIL);
    expect(clampListWidth(500, 600)).toBe(MIN_LIST); // tiny window: list min wins
    // UNI-218 F13: one default list width for every list | detail screen but the board.
    for (const screen of ['sessions', 'chat', 'files', 'unknown']) expect(defaultListWidth(screen, 1216)).toBe(DEFAULT_LIST_WIDTH);
    expect(DEFAULT_LIST_WIDTH).toBe(400);
    expect(defaultListWidth('board', 1216)).toBe(1216 - 560);
  });
});
