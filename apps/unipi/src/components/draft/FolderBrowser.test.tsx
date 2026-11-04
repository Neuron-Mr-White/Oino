// @vitest-environment jsdom
/**
 * The workspace folder browser pane (owner-fix-5): Miller columns over
 * `fs_list` (one call per column, cached), a Places rail, a breadcrumb path
 * bar with a type-a-path mode, a filter for the active column, the `.git`
 * pill, and the footer's [System dialog…] · Cancel · Use this folder.
 * `fsList` is mocked — this suite pins the browser's own behaviour.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPlatformOverride } from '../../lib/platform';
import { setDesktopOverride } from '../../lib/layout';
import { FolderBrowser } from './FolderBrowser';
import { WorkspacePicker } from './WorkspacePicker';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const fs = vi.hoisted(() => ({ list: vi.fn() }));
vi.mock('../../lib/fs', () => ({ fsList: (...args: unknown[]) => fs.list(...args) }));

const entry = (name: string, kind: 'dir' | 'file' = 'dir') => ({ name, kind, size: 0, mtime: 0 });

/** A few levels deep, one branch with `.git`, one wide (the visual check uses 60). */
const TREE: Record<string, string[]> = {
  '/home/you': ['p', 'z'],
  '/home/you/p': ['oino', 'web'],
  '/home/you/p/oino': ['src'],
  '/home/you/p/web': [],
  '/home/you/z': ['src'],
  '/home/you/z/src': ['.git', 'notes.md'],
  '/srv': ['data'],
  '/srv/data': [],
};

beforeEach(() => {
  fs.list.mockImplementation((_host: string, path: string) => {
    const names = TREE[path];
    if (!names) return Promise.resolve({ ok: false, error: { code: 'E_SCOPE', message: 'outside the allowed folders', retryable: false } });
    return Promise.resolve({
      ok: true,
      value: { entries: names.map((n) => entry(n, n === 'notes.md' ? 'file' : 'dir')), more: false },
    });
  });
});

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
});

const boot = (over: Partial<Parameters<typeof FolderBrowser>[0]> = {}) => {
  const onUse = vi.fn();
  const onClose = vi.fn();
  render(() => (
    <FolderBrowser
      hostId="h"
      home="/home/you"
      roots={['/home/you', '/srv']}
      recentItems={[{ path: '/home/you/p/oino', label: 'oino' }]}
      ownPc={false}
      onUse={onUse}
      onClose={onClose}
      {...over}
    />
  ));
  return { onUse, onClose };
};

const rows = (col: number) => [...document.querySelectorAll(`[data-col="${col}"] .fb-row`)] as HTMLElement[];
const footPath = () => document.querySelector('.fb-foot-path')?.textContent;

describe('FolderBrowser', () => {
  it('opens at `initial` with one column per ancestor and lists them (one fs_list per column)', async () => {
    boot({ initial: '/home/you/p/oino' });
    await waitFor(() => expect(rows(2).length).toBe(1));
    expect(rows(0).map((r) => r.textContent)).toEqual(['p', 'z']);
    expect(rows(1).map((r) => r.textContent)).toEqual(['oino', 'web']);
    expect(rows(2).map((r) => r.textContent)).toEqual(['src']);
    expect(footPath()).toBe('~/p/oino');
    const listed = fs.list.mock.calls.map((c) => c[1]);
    expect(listed).toEqual(['/home/you', '/home/you/p', '/home/you/p/oino']);
  });

  it('clicking folders adds columns and the footer path follows; Use passes the deepest folder', async () => {
    const { onUse } = boot();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.click(rows(0)[0]!); // p
    await waitFor(() => expect(rows(1).length).toBeGreaterThan(0));
    expect(footPath()).toBe('~/p');
    fireEvent.click(rows(1)[0]!); // oino
    await waitFor(() => expect(rows(2).length).toBeGreaterThan(0));
    expect(footPath()).toBe('~/p/oino');
    fireEvent.click(screen.getByTestId('workspace-use'));
    await waitFor(() => expect(onUse).toHaveBeenCalledWith('/home/you/p/oino'));
  });

  it('Enter in the path input jumps; a bad path says so calmly and keeps the pane', async () => {
    boot();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.click(screen.getByTestId('workspace-crumbs'));
    const input = await waitFor(() => screen.getByTestId('workspace-path-edit') as HTMLInputElement);
    expect(input.value).toBe('/home/you');
    fireEvent.input(input, { target: { value: '/srv/data' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(screen.getByTestId('workspace-col-1')).toBeTruthy());
    expect(footPath()).toBe('/srv/data');
    // Now a path the fixture doesn't know: the calm scope message, inline.
    fireEvent.click(screen.getByTestId('workspace-crumbs'));
    const input2 = await waitFor(() => screen.getByTestId('workspace-path-edit') as HTMLInputElement);
    fireEvent.input(input2, { target: { value: '/nope' } });
    fireEvent.keyDown(input2, { key: 'Enter' });
    await waitFor(() => expect(screen.getByTestId('workspace-path-error')).toBeTruthy());
    expect(screen.getByTestId('workspace-path-error').textContent).toContain('Unipi can’t open this folder');
    // Esc reverts the edit mode; the pane stays.
    fireEvent.keyDown(input2, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('workspace-path-edit')).toBeNull());
    expect(screen.getByTestId('workspace-browser')).toBeTruthy();
  });

  it('the filter narrows the active column', async () => {
    boot();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    const filter = screen.getByTestId('workspace-filter') as HTMLInputElement;
    fireEvent.click(rows(0)[0]!); // p — the active column is now p's listing
    await waitFor(() => expect(screen.getByTestId('workspace-col-1')).toBeTruthy());
    fireEvent.input(filter, { target: { value: 'web' } });
    await waitFor(() => expect(rows(1).map((r) => r.textContent)).toEqual(['web']));
    fireEvent.input(filter, { target: { value: 'zzz' } });
    await waitFor(() => expect(document.querySelector('[data-col="1"]')!.textContent).toContain('No folder matches'));
  });

  it('a `.git` folder in the listing shows the pill on the row and in the footer', async () => {
    boot();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.click(rows(0)[1]!); // z
    await waitFor(() => expect(rows(1).length).toBeGreaterThan(0));
    fireEvent.click(rows(1)[0]!); // src — its listing contains a .git dir
    // The pill rides src's row (in its parent column) and the footer; src itself shows
    // only dot/file entries, so its column says "No folders inside".
    await waitFor(() => expect(screen.getByTestId('workspace-git-pill')).toBeTruthy());
    expect(rows(1)[0]!.textContent).toContain('git');
    expect(footPath()).toBe('~/z/src');
  });

  it('Escape closes the pane', async () => {
    const { onClose } = boot();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.keyDown(screen.getByTestId('workspace-browser'), { key: 'Escape' });
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  });

  it('keyboard: ↓ selects, → opens the next column, ← goes back (selection kept), Ctrl+Enter uses', async () => {
    const { onUse } = boot();
    const dialog = await waitFor(() => screen.getByTestId('workspace-browser'));
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.keyDown(dialog, { key: 'ArrowDown' });
    expect(rows(0)[0]!.hasAttribute('data-sel')).toBe(true);
    fireEvent.keyDown(dialog, { key: 'ArrowRight' });
    await waitFor(() => expect(rows(1).length).toBeGreaterThan(0));
    fireEvent.keyDown(dialog, { key: 'ArrowLeft' });
    await waitFor(() => expect(screen.queryByTestId('workspace-col-1')).toBeNull());
    // Back at one column, the selection is kept: ↓ moves to the next row.
    expect(rows(0)[0]!.hasAttribute('data-sel')).toBe(true);
    fireEvent.keyDown(dialog, { key: 'ArrowDown' });
    expect(rows(0)[1]!.hasAttribute('data-sel')).toBe(true);
    fireEvent.keyDown(dialog, { key: 'Enter' });
    await waitFor(() => expect(rows(1).length).toBeGreaterThan(0));
    fireEvent.keyDown(dialog, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(onUse).toHaveBeenCalled());
    expect(onUse.mock.calls[0]![0]).toBe('/home/you/z');
  });

  it('the System dialog button appears only on the own PC', async () => {
    const { unmount } = render(() => (
      <FolderBrowser hostId="h" home="/home/you" roots={[]} recentItems={[]} ownPc={false} onClose={() => {}} onUse={() => {}} />
    ));
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    expect(screen.queryByTestId('workspace-sysdlg')).toBeNull();
    unmount();
    cleanup();
    setPlatformOverride('tauri-desktop');
    render(() => (
      <FolderBrowser hostId="h" home="/home/you" roots={[]} recentItems={[]} ownPc={true} onClose={() => {}} onUse={() => {}} />
    ));
    await waitFor(() => expect(screen.getByTestId('workspace-sysdlg')).toBeTruthy());
  });

  it('Ctrl+L opens the path editor and the empty bar click does too', async () => {
    boot();
    const dialog = await waitFor(() => screen.getByTestId('workspace-browser'));
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.keyDown(dialog, { key: 'l', ctrlKey: true });
    expect(screen.getByTestId('workspace-path-edit')).toBeTruthy();
    fireEvent.keyDown(screen.getByTestId('workspace-path-edit'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('workspace-path-edit')).toBeNull());
    fireEvent.click(screen.getByTestId('workspace-crumbs'));
    expect(screen.getByTestId('workspace-path-edit')).toBeTruthy();
  });
});

describe('WorkspacePicker → FolderBrowser (desktop)', () => {
  it('Browse folders… opens the pane on the own PC too — the native dialog stays a footer button', async () => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    // `invoke` rejects: a native-dialog attempt would surface as a fallthrough; the pane must open.
    render(() => (
      <WorkspacePicker hostId="local" value={undefined} onChange={() => {}} recent={[]} roots={['/home/you']} home="/home/you" />
    ));
    await waitFor(() => expect(screen.getByTestId('workspace-chip')).toBeTruthy());
    fireEvent.click(screen.getByTestId('workspace-chip'));
    fireEvent.click(await screen.findByTestId('workspace-open'));
    await waitFor(() => expect(screen.getByTestId('workspace-browser')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('workspace-sysdlg')).toBeTruthy());
  });

  it('Browse folders… closes the chip menu and the portaled pane takes mouse clicks (owner-fix-6)', async () => {
    setDesktopOverride(true);
    render(() => (
      <div class="draft" style={{ isolation: 'isolate' }}>
        <WorkspacePicker hostId="local" value={undefined} onChange={() => {}} recent={[]} roots={['/home/you']} home="/home/you" />
      </div>
    ));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    fireEvent.click(await screen.findByTestId('workspace-open'));
    const dialog = await waitFor(() => screen.getByTestId('workspace-browser'));
    expect(screen.queryByTestId('workspace-menu')).toBeNull();
    expect(dialog.closest('.draft')).toBeNull();
    await waitFor(() => expect(rows(0).length).toBeGreaterThan(0));
    fireEvent.click(rows(0)[0]!);
    expect(screen.getByTestId('workspace-browser')).toBeTruthy();
  });
});
