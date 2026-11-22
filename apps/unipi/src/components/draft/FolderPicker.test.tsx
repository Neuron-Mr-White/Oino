// @vitest-environment jsdom
/**
 * The one workspace picker (owner spec 2026-11-19): a regular folder dialog with zoxide underneath.
 * `fs_list` / `fs_zoxide` are mocked — this suite pins the behaviour: focus on open, Frequent from
 * zoxide merged with recents, type-to-find (Enter opens zoxide's top hit), path mode with Tab, the
 * keyboard map, browsing, the no-zoxide fallback and the phone sheet.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';

const fs = vi.hoisted(() => ({ list: vi.fn(), roots: vi.fn(), zoxide: vi.fn() }));
vi.mock('../../lib/fs', () => ({
  fsList: (...a: unknown[]) => fs.list(...a),
  fsRoots: (...a: unknown[]) => fs.roots(...a),
  fsZoxide: (...a: unknown[]) => fs.zoxide(...a),
}));
const local = vi.hoisted(() => ({ zoxide: vi.fn(), native: vi.fn() }));
vi.mock('../../lib/openLocal', () => ({
  isOwnPcFile: () => false,
  localZoxide: (...a: unknown[]) => local.zoxide(...a),
  pickFolderNative: (...a: unknown[]) => local.native(...a),
}));
const platform = vi.hoisted(() => ({ tauri: false }));
vi.mock('../../lib/platform', async (orig) => ({ ...(await orig<typeof import('../../lib/platform')>()), isTauriDesktop: () => platform.tauri }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

import { FolderPicker } from './FolderPicker';

const HOME = '/home/you';
const TREE: Record<string, string[]> = {
  '/': ['home', 'srv'],
  '/home': ['you'],
  [HOME]: ['Projects', 'notes', 'oinotes'],
  [`${HOME}/Projects`]: ['oino', 'oino-web', 'oink', 'site'],
  [`${HOME}/Projects/oino`]: ['src', 'docs'],
  [`${HOME}/Projects/oino/docs`]: [],
};
const ZOXIDE = [
  { path: `${HOME}/Projects/site`, score: 40 },
  { path: `${HOME}/Projects/oino`, score: 90 },
  { path: `${HOME}/Projects/oino-web`, score: 20 },
  { path: '/srv/oil', score: 5 },
];
const RECENTS = [
  { path: `${HOME}/Projects/site`, label: 'site', detail: '~/Projects/site' },
  { path: `${HOME}/old/legacy`, label: 'legacy', detail: '~/old/legacy' },
];

/** `zoxide query <words>` the way the host answers it: every word a substring in order, best score first. */
const zoxideFor = (query: string, limit = 20) => {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const hits = ZOXIDE.filter((h) => {
    let at = 0;
    return words.every((w) => {
      const i = h.path.toLowerCase().indexOf(w, at);
      at = i + w.length;
      return i >= 0;
    }) && h.path.toLowerCase().split('/').pop()!.includes(words[words.length - 1] ?? '');
  })
    .sort((a, b) => b.score - a.score)
    .slice(0, limit);
  return { ok: true, value: { available: true, hits } };
};

const boot = (over: Partial<Parameters<typeof FolderPicker>[0]> = {}) => {
  const onUse = vi.fn();
  const onClose = vi.fn();
  const onNone = vi.fn();
  render(() => <FolderPicker hostId="pc" home={HOME} roots={[HOME, '/srv']} recentItems={RECENTS} allowNone onNone={onNone} onUse={onUse} onClose={onClose} {...over} />);
  return { onUse, onClose, onNone };
};

const input = () => screen.getByTestId('workspace-input') as HTMLInputElement;
const type = (text: string) => fireEvent.input(input(), { target: { value: text } });
const key = (k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(input(), { key: k, ...init });
const rowsOf = () => [...screen.getByTestId('workspace-list').querySelectorAll<HTMLElement>('[role=option]')];
const paths = () => rowsOf().map((r) => r.getAttribute('title'));
const selected = () => rowsOf().find((r) => r.getAttribute('aria-selected') === 'true')?.getAttribute('title');
const groups = () => [...screen.getByTestId('workspace-list').querySelectorAll('[data-testid=workspace-group]')].map((g) => g.textContent);
const settle = (ms = 150) => new Promise((r) => setTimeout(r, ms));

beforeEach(() => {
  setDesktopOverride(true);
  platform.tauri = false;
  fs.roots.mockResolvedValue({ ok: true, value: { roots: [HOME] } });
  fs.list.mockImplementation(async (_h: string, path: string) => {
    const names = TREE[path];
    return names ? { ok: true, value: { entries: names.map((name) => ({ name, kind: 'dir' })) } } : { ok: false, error: { message: `No folder ${path}` } };
  });
  fs.zoxide.mockImplementation(async (_h: string, q: string, limit?: number) => zoxideFor(q, limit));
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  for (const m of [fs.list, fs.roots, fs.zoxide, local.zoxide, local.native]) m.mockReset();
});

describe('one picker, regular folder dialog', () => {
  it('is one dialog with a focused location box, an Up button, one list, Use this folder and Cancel', async () => {
    boot();
    await screen.findByTestId('workspace-browser');
    await waitFor(() => expect(document.activeElement).toBe(input()));
    expect(screen.getByTestId('workspace-up')).toBeTruthy();
    expect(screen.getByTestId('workspace-use').textContent).toContain('Use');
    expect(screen.getByText('Cancel')).toBeTruthy();
    // ONE list, no Miller columns, no places rail.
    expect(document.querySelectorAll('[role=listbox]')).toHaveLength(1);
    expect(document.querySelector('.fb-columns, .fb-places, [data-testid=workspace-columns]')).toBeNull();
  });

  it('empty box: Frequent in zoxide order (deduplicated with recents), then the current folder\'s folders', async () => {
    boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(5));
    expect(groups()).toEqual(['Frequent', `In ~/`]);
    expect(paths()).toEqual([
      `${HOME}/Projects/oino`,
      `${HOME}/Projects/site`,
      `${HOME}/Projects/oino-web`,
      '/srv/oil',
      `${HOME}/old/legacy`, // an Oino recent zoxide did not list
      `${HOME}/notes`,
      `${HOME}/oinotes`,
      `${HOME}/Projects`,
    ]);
    // `site` is in both lists: shown once, in zoxide's slot.
    expect(paths().filter((p) => p === `${HOME}/Projects/site`)).toHaveLength(1);
    // The project you use most is highlighted: Enter opens it.
    expect(selected()).toBe(`${HOME}/Projects/oino`);
  });

  it('Enter on the highlighted Frequent folder opens it as the workspace', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(selected()).toBe(`${HOME}/Projects/oino`));
    key('Enter');
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino`);
  });
});

describe('type to find (zoxide)', () => {
  it('typing "oin" ranks like zoxide query and Enter opens the top hit', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('oin');
    await waitFor(() => expect(fs.zoxide).toHaveBeenCalledWith('pc', 'oin', 20));
    await waitFor(() => expect(groups()).toContain('Best matches'));
    expect(paths().slice(0, 2)).toEqual([`${HOME}/Projects/oino`, `${HOME}/Projects/oino-web`]);
    expect(selected()).toBe(`${HOME}/Projects/oino`);
    key('Enter');
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino`);
  });

  it('Enter straight after typing (before the debounce) still opens zoxide\'s top hit, not a stale row', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('oin');
    key('Enter');
    await waitFor(() => expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino`));
  });

  it('several words go to zoxide together', async () => {
    boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('proj web');
    await waitFor(() => expect(fs.zoxide).toHaveBeenCalledWith('pc', 'proj web', 20));
    await waitFor(() => expect(paths()[0]).toBe(`${HOME}/Projects/oino-web`));
  });

  it('after zoxide\'s hits: matching recents, then matching folders right here — all of them open', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('le');
    // N5: with zoxide the merged recents read "Frequent" too (never "Recent").
    await waitFor(() => expect(groups()).toContain('Frequent'));
    expect(groups()).not.toContain('Recent');
    expect(paths()).toContain(`${HOME}/old/legacy`);
    type('oinot');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/oinotes`]));
    expect(rowsOf()[0]!.getAttribute('data-source')).toBe('near');
    // A plain folder zoxide doesn't know is ENTERED on Enter (type-ahead), not made the workspace.
    key('Enter');
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/oinotes'));
    expect(onUse).not.toHaveBeenCalled();
  });

  it('says so when nothing matches, and Enter does nothing', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('zzzz');
    await waitFor(() => expect(screen.getByTestId('workspace-note').textContent).toContain('No folder matches'));
    key('Enter');
    await settle();
    expect(onUse).not.toHaveBeenCalled();
  });
});

describe('path mode', () => {
  it('~/ switches to path navigation: the folder is listed and the last segment filters it', async () => {
    boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('~/Projects/');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/Projects/oink`, `${HOME}/Projects/oino`, `${HOME}/Projects/oino-web`, `${HOME}/Projects/site`]));
    expect(screen.getByTestId('workspace-picker').getAttribute('data-mode')).toBe('path');
    expect(fs.zoxide).not.toHaveBeenCalledWith('pc', '~/Projects/', expect.anything());
    type('~/Projects/oin');
    await waitFor(() => expect(paths().sort()).toEqual([`${HOME}/Projects/oink`, `${HOME}/Projects/oino`, `${HOME}/Projects/oino-web`]));
    expect(selected()).toBe(paths()[0]);
  });

  it('Tab completes (shared prefix, then the highlighted folder), Enter enters it, Ctrl+Enter uses it', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('~/Pro');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/Projects`]));
    key('Tab');
    expect(input().value).toBe('~/Projects/');
    await waitFor(() => expect(paths()[0]).toBe(`${HOME}/Projects/oink`));
    type('~/Projects/oi');
    await waitFor(() => expect(paths()).toHaveLength(3));
    key('Tab');
    expect(input().value).toBe('~/Projects/oin');
    // The shared prefix cannot grow: Tab completes to the highlighted match.
    await waitFor(() => expect(paths()).toHaveLength(3));
    for (let i = 0; i < 4 && selected() !== `${HOME}/Projects/oino`; i += 1) key('ArrowDown');
    expect(selected()).toBe(`${HOME}/Projects/oino`);
    key('Tab');
    expect(input().value).toBe('~/Projects/oino/');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/Projects/oino/docs`, `${HOME}/Projects/oino/src`]));
    // Nothing highlighted yet: the box's folder is the target. Enter enters the highlighted folder.
    expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects/oino');
    key('ArrowDown');
    expect(selected()).toBe(`${HOME}/Projects/oino/docs`);
    key('Enter');
    await waitFor(() => expect(input().value).toBe(''));
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects/oino/docs'));
    expect(onUse).not.toHaveBeenCalled();
    key('Enter', { ctrlKey: true });
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino/docs`);
  });

  it('./ and ../ resolve against the folder being browsed', async () => {
    boot({ initial: `${HOME}/Projects` });
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/site`));
    type('../');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/notes`, `${HOME}/oinotes`, `${HOME}/Projects`]));
    type('./oi');
    await waitFor(() => expect(paths().sort()).toEqual([`${HOME}/Projects/oink`, `${HOME}/Projects/oino`, `${HOME}/Projects/oino-web`]));
  });

  it('a folder it cannot list reads calmly', async () => {
    fs.list.mockImplementation(async () => ({ ok: false, error: { message: '/etc is outside the allowed folders ($HOME=…)' } }));
    boot();
    type('/etc/');
    await waitFor(() => expect(screen.getByTestId('workspace-note').textContent).toBe('Unipi may only browse your home folder and the folders this PC allows. Go back to one of them.'));
  });
});

describe('browsing', () => {
  it('double-click (or Enter) enters a folder; Backspace / Up go to the parent; Use this folder uses the current one', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects`));
    const projects = rowsOf().find((r) => r.getAttribute('title') === `${HOME}/Projects` && r.getAttribute('data-source') === 'dir')!;
    fireEvent.click(projects);
    expect(projects.getAttribute('aria-selected')).toBe('true');
    fireEvent.dblClick(projects);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects');
    // Enter on a browsed folder ENTERS it.
    key('ArrowDown');
    key('ArrowDown');
    expect(selected()).toBe(`${HOME}/Projects/oino`);
    key('Enter');
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oino/docs`));
    expect(onUse).not.toHaveBeenCalled();
    // Backspace on the empty box goes up, so does the Up button.
    key('Backspace');
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects'));
    fireEvent.click(screen.getByTestId('workspace-up'));
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~'));
    fireEvent.dblClick(rowsOf().find((r) => r.getAttribute('title') === `${HOME}/Projects` && r.getAttribute('data-source') === 'dir')!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    fireEvent.click(screen.getByTestId('workspace-use'));
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects`);
  });

  it('Ctrl/Cmd+Enter uses the highlighted browsed folder instead of entering it', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects`));
    fireEvent.dblClick(rowsOf().find((r) => r.getAttribute('title') === `${HOME}/Projects` && r.getAttribute('data-source') === 'dir')!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    key('ArrowDown');
    key('ArrowDown');
    expect(selected()).toBe(`${HOME}/Projects/oino`);
    key('Enter', { metaKey: true });
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino`);
  });

  it('the breadcrumbs jump to an ancestor', async () => {
    boot({ initial: `${HOME}/Projects/oino` });
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oino/src`));
    fireEvent.click(screen.getByTestId('workspace-crumbs').querySelectorAll('button')[1]!);
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects'));
  });

  it('Esc clears the box first, then closes', async () => {
    const { onClose } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('oin');
    key('Escape');
    expect(input().value).toBe('');
    expect(onClose).not.toHaveBeenCalled();
    key('Escape');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('keeps No workspace as an option, and the system dialog only on own PC in the desktop app', async () => {
    const { onNone } = boot();
    fireEvent.click(screen.getByTestId('workspace-none'));
    expect(onNone).toHaveBeenCalled();
    expect(screen.queryByTestId('workspace-sysdlg')).toBeNull();
    cleanup();
    platform.tauri = true;
    const { onUse } = boot({ ownPc: true, allowNone: false, onNone: undefined });
    expect(screen.queryByTestId('workspace-none')).toBeNull();
    local.native.mockResolvedValue('/home/you/elsewhere');
    fireEvent.click(screen.getByTestId('workspace-sysdlg'));
    await waitFor(() => expect(onUse).toHaveBeenCalledWith('/home/you/elsewhere'));
  });
});

describe('without zoxide', () => {
  beforeEach(() => {
    fs.zoxide.mockImplementation(async () => ({ ok: true, value: { available: false, hits: [] } }));
  });

  it('still a normal folder browser: Recent instead of Frequent, a one-line hint, no error', async () => {
    const { onUse } = boot();
    await screen.findByTestId('workspace-zoxide-hint');
    expect(screen.getByTestId('workspace-zoxide-hint').textContent).toMatch(/zoxide/);
    expect(groups()).toEqual(['Recent', 'In ~/']);
    expect(paths().slice(0, 2)).toEqual([`${HOME}/Projects/site`, `${HOME}/old/legacy`]);
    expect(screen.queryByRole('alert')).toBeNull();
    // Browsing works.
    fireEvent.dblClick(rowsOf().find((r) => r.getAttribute('data-source') === 'dir' && r.getAttribute('title') === `${HOME}/Projects`)!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    // Typing falls back to recents and nearby folders.
    type('leg');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/old/legacy`]));
    expect(screen.getByTestId('workspace-status').textContent).toMatch(/recent and nearby/i);
    key('Enter');
    await waitFor(() => expect(onUse).toHaveBeenCalledWith(`${HOME}/old/legacy`));
  });

  it('an older host that does not know the op reads the same (no error); the own PC asks local zoxide', async () => {
    fs.zoxide.mockImplementation(async () => ({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown op' } }));
    boot();
    await screen.findByTestId('workspace-zoxide-hint');
    expect(local.zoxide).not.toHaveBeenCalled();
  });
});

describe('phone: the same picker as a full-screen sheet', () => {
  beforeEach(() => setDesktopOverride(false));

  it('renders in the sheet with Frequent, taps open a Frequent folder and enter a browsed one', async () => {
    const { onUse } = boot();
    await screen.findByTestId('workspace-sheet');
    expect(screen.queryByTestId('workspace-browser')).toBeNull();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(5));
    expect(groups()[0]).toBe('Frequent');
    // Nothing is pre-highlighted on touch.
    expect(selected()).toBeUndefined();
    // Tap a browsed folder: enters it.
    fireEvent.click(rowsOf().find((r) => r.getAttribute('data-source') === 'dir' && r.getAttribute('title') === `${HOME}/Projects`)!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    expect(onUse).not.toHaveBeenCalled();
    // The footer's primary uses the folder you are in.
    fireEvent.click(screen.getByTestId('workspace-use'));
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects`);
  });

  it('a tap on a Frequent / search row opens it; the host\'s zoxide answers the search', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(5));
    type('oin');
    await waitFor(() => expect(fs.zoxide).toHaveBeenCalledWith('pc', 'oin', 20));
    await waitFor(() => expect(groups()).toContain('Best matches'));
    fireEvent.click(rowsOf()[0]!);
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects/oino`);
  });

  it('shows No workspace and has no System dialog', async () => {
    const { onNone } = boot();
    await screen.findByTestId('workspace-sheet');
    fireEvent.click(screen.getByTestId('workspace-none'));
    expect(onNone).toHaveBeenCalled();
    expect(screen.queryByTestId('workspace-sysdlg')).toBeNull();
  });
});

describe('QA follow-up (strict acceptance)', () => {
  it('P1-1: type, Tab, Enter never picks "No workspace": Tab keeps focus in the box', async () => {
    const { onNone, onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('oin');
    await waitFor(() => expect(groups()).toContain('Best matches'));
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    input().dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(input());
    // Tab stepped inside the highlighted match (it is not opened, not "No workspace").
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects/oino'));
    expect(input().value).toBe('');
    expect(onNone).not.toHaveBeenCalled();
    expect(onUse).not.toHaveBeenCalled();
    key('Enter');
    expect(onNone).not.toHaveBeenCalled();
  });

  it('P1-1: Tab with nothing to step into is swallowed too', async () => {
    const { onNone } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('zzzz');
    const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    input().dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(input());
    key('Enter');
    expect(onNone).not.toHaveBeenCalled();
  });

  it('P2-2: Esc clears then closes from anywhere — header and footer buttons included', async () => {
    const { onClose } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('oin');
    const cancel = screen.getByText('Cancel');
    cancel.focus();
    fireEvent.keyDown(cancel, { key: 'Escape' });
    expect(input().value).toBe('');
    expect(onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(input());
    for (const el of [screen.getByTestId('workspace-browser-close'), screen.getByTestId('workspace-none'), screen.getByTestId('workspace-use')]) {
      onClose.mockClear();
      el.focus();
      fireEvent.keyDown(el, { key: 'Escape' });
      expect(onClose).toHaveBeenCalledTimes(1);
    }
  });

  it('P2-3: focus returns to the box after the system dialog', async () => {
    platform.tauri = true;
    boot({ ownPc: true });
    local.native.mockResolvedValue(null);
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    const btn = screen.getByTestId('workspace-sysdlg');
    btn.focus();
    fireEvent.click(btn);
    await waitFor(() => expect(document.activeElement).toBe(input()));
  });

  it('P2-1: the box is focused on open in the narrow layout too; only the Android/iOS app leaves the keyboard down', async () => {
    setDesktopOverride(false);
    boot();
    await screen.findByTestId('workspace-sheet');
    await waitFor(() => expect(document.activeElement).toBe(input()));
    cleanup();
    setPlatformOverride('tauri-mobile');
    try {
      boot();
      await screen.findByTestId('workspace-sheet');
      await settle(350);
      expect(document.activeElement).not.toBe(input());
      // …and a tap on the box still works as the search box.
      input().focus();
      type('oin');
      await waitFor(() => expect(groups()).toContain('Best matches'));
    } finally {
      setPlatformOverride(undefined);
    }
  });

  it('P2-5: typing a plain folder name and Enter enters it (type-ahead); Enter then uses the folder you are in', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects`));
    type('Proj');
    await waitFor(() => expect(rowsOf()[0]?.getAttribute('title')).toBe(`${HOME}/Projects`));
    key('Enter');
    await waitFor(() => expect(input().value).toBe(''));
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    expect(screen.getByTestId('workspace-target').textContent).toBe('~/Projects');
    expect(onUse).not.toHaveBeenCalled();
    key('Enter');
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects`);
  });

  it('P2-6: a parent folder above frequent projects is found by name (zoxide only matches the last component)', async () => {
    fs.zoxide.mockImplementation(async (_h: string, q: string) =>
      q ? { ok: true, value: { available: true, hits: [] } } : { ok: true, value: { available: true, hits: [{ path: `${HOME}/Work/demoeditor/api`, score: 9 }, { path: `${HOME}/Work/demoeditor/web`, score: 5 }] } },
    );
    boot();
    await waitFor(() => expect(groups()[0]).toBe('Frequent'));
    type('de');
    await waitFor(() => expect(paths()).toContain(`${HOME}/Work/demoeditor`));
    expect(paths().filter((p) => p === `${HOME}/Work/demoeditor`)).toHaveLength(1);
    key('Enter');
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~/Work/demoeditor'));
  });

  it('P3-2: symlinked folders are listed', async () => {
    fs.list.mockImplementation(async () => ({ ok: true, value: { entries: [{ name: 'Projects', kind: 'dir' }, { name: 'work-link', kind: 'symlink' }, { name: 'a.txt', kind: 'file' }] } }));
    boot();
    await waitFor(() => expect(paths()).toContain(`${HOME}/work-link`));
    expect(paths()).not.toContain(`${HOME}/a.txt`);
  });

  it('P3-7: going back into the start folder does not bring Frequent back above its contents', async () => {
    boot();
    await waitFor(() => expect(groups()[0]).toBe('Frequent'));
    fireEvent.dblClick(rowsOf().find((r) => r.getAttribute('data-source') === 'dir' && r.getAttribute('title') === `${HOME}/Projects`)!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    key('Backspace');
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~'));
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects`));
    expect(groups()).toEqual([]);
  });

  it('Tab in a Frequent list steps inside the highlighted project (to pick a subfolder)', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(selected()).toBe(`${HOME}/Projects/oino`));
    key('Tab');
    await waitFor(() => expect(paths()).toEqual([`${HOME}/Projects/oino/docs`, `${HOME}/Projects/oino/src`]));
    expect(onUse).not.toHaveBeenCalled();
  });

  it('shows the caller\'s error (e.g. the Board refusing a folder) and a custom action label', async () => {
    boot({ error: 'Not a git repo', actionLabel: 'Add', title: 'Add a project', allowNone: false, onNone: undefined });
    expect(screen.getByTestId('workspace-error').textContent).toBe('Not a git repo');
    expect(screen.getByTestId('workspace-use').textContent).toContain('Add');
    expect(screen.getByRole('dialog').getAttribute('aria-label')).toBe('Add a project');
    expect(screen.queryByTestId('workspace-none')).toBeNull();
  });
});

describe('N1: the whole home folder (or /) is never a workspace by a key', () => {
  it('Backspace to ~ with nothing highlighted: Enter and Ctrl+Enter do nothing; only a click on "Use this folder" takes it', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(groups()[0]).toBe('Frequent'));
    fireEvent.dblClick(rowsOf().find((r) => r.getAttribute('data-source') === 'dir' && r.getAttribute('title') === `${HOME}/Projects`)!);
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    key('Backspace');
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~'));
    expect(selected()).toBeUndefined();
    key('Enter');
    key('Enter', { ctrlKey: true });
    await settle();
    expect(onUse).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('workspace-use'));
    expect(onUse).toHaveBeenCalledWith(HOME);
  });

  it('~/ + Enter lists home (enters the typed folder) instead of using it', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('~/');
    await waitFor(() => expect(screen.getByTestId('workspace-input').closest('.fp')!.getAttribute('data-mode')).toBe('path'));
    key('Enter');
    await waitFor(() => expect(input().value).toBe(''));
    await waitFor(() => expect(screen.getByTestId('workspace-target').textContent).toBe('~'));
    expect(onUse).not.toHaveBeenCalled();
  });

  it('/ + Enter enters the root; the root is used only by the button', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rowsOf().length).toBeGreaterThan(0));
    type('/');
    await waitFor(() => expect(paths()).toEqual([`/home`, `/srv`]));
    key('Enter');
    await waitFor(() => expect(input().value).toBe(''));
    expect(screen.getByTestId('workspace-target').textContent).toBe('/');
    key('Enter', { ctrlKey: true });
    await settle();
    expect(onUse).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('workspace-use'));
    expect(onUse).toHaveBeenCalledWith('/');
  });

  it('a highlighted folder and any other folder you are in still work from the keyboard', async () => {
    const { onUse } = boot({ initial: `${HOME}/Projects` });
    await waitFor(() => expect(paths()).toContain(`${HOME}/Projects/oink`));
    key('Enter', { ctrlKey: true });
    expect(onUse).toHaveBeenCalledWith(`${HOME}/Projects`);
  });
});

describe('N2: a long folder name keeps the footer on one line', () => {
  const LONG = 'a-project-with-an-extremely-long-folder-name-that-keeps-going-and-going-for-seventy-five-chars';
  it('the primary button and the footer path are cut in the middle: the whole name stays in the DOM, its end is its own span', async () => {
    fs.zoxide.mockImplementation(async (_h: string, q: string) => ({ ok: true, value: { available: true, hits: q ? [] : [{ path: `${HOME}/Projects/${LONG}`, score: 99 }] } }));
    boot();
    await waitFor(() => expect(selected()).toBe(`${HOME}/Projects/${LONG}`));
    const use = screen.getByTestId('workspace-use');
    expect(use.textContent).toBe(`Use “${LONG}”`);
    const tail = use.querySelector('.mid-tail')!.textContent!;
    expect(LONG.endsWith(tail)).toBe(true);
    expect(tail.length).toBeLessThan(LONG.length / 2);
    const foot = screen.getByTestId('workspace-target');
    expect(foot.textContent).toBe(`~/Projects/${LONG}`);
    expect(foot.querySelector('.mid-tail')!.textContent).toBe(LONG.slice(-16));
  });
});
