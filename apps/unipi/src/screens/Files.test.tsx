// @vitest-environment jsdom
import { z } from 'zod';
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Files from './Files';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';
import { setChatTransport, type SessionListData } from '../lib/session/link';
import { consumeMentionFor } from '../lib/chat/mention';
import { getEditorTarget, clearEditorTarget } from '../lib/editorTarget';
import { setBlobTransport, type BlobTransport } from '../lib/blob';
import { setPlatformOverride } from '../lib/platform';
import { resetLayout } from '../lib/layout';
import { setFilesTarget } from '../lib/filesTarget';
import { closeGit, gitTarget } from '../lib/git';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

function chatListing(panes: Array<{ id: string; cwd: string }>): SessionListData {
  return { sessions: panes.map((p) => ({ id: p.id, title: p.id, cwd: p.cwd, state: 'idle' as const })) };
}

const hosts = [{ id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

const listing = {
  entries: [
    { name: 'b.txt', kind: 'file', size: 10, mtime: 1700000000000, ignored: false },
    { name: 'src', kind: 'dir', size: 0, mtime: 1700000000000, ignored: false },
    { name: 'a.txt', kind: 'file', size: 20, mtime: 1700000000000, git: 'M', ignored: false },
    { name: 'zdir', kind: 'dir', size: 0, mtime: 1700000000000, ignored: false },
  ],
  more: false,
};

function mockIpc() {
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: hosts };
    if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u/proj'] } };
    if (cmd === 'fs_list') return { ok: true, value: listing };
    if (cmd === 'fs_stat') return { ok: false, error: { code: 'E_NOT_FOUND', message: 'not found', retryable: false } };
    // Host replies are schema-checked behind the HostClient seam (WP-A1): a real FsWriteData.
    if (cmd === 'fs_write') return { ok: true, value: { hash: 'h0', mtime: 0 } };
    return { ok: true, value: null };
  });
}

function mount() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Files /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.resetAllMocks();
  setChatTransport(undefined);
  location.hash = '#/';
});

beforeEach(() => mockIpc());

describe('Files screen (UNI-155)', () => {
  it('no PC paired: the shared no-host state, no fake "This PC" chrome (UNI-218 F36)', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => (cmd === 'list_hosts' ? { ok: true, value: [] } : { ok: true, value: null }));
    mount();
    expect(await screen.findByTestId('no-host-state')).toHaveTextContent('No PC paired yet');
    expect(screen.getByRole('button', { name: 'Add a PC' })).toBeInTheDocument();
    expect(screen.queryByText('This PC')).toBeNull();
    expect(screen.queryByText('Hidden')).toBeNull();
    expect(screen.queryByLabelText('Files menu')).toBeNull();
  });

  it('header: the shared PC chip names the PC; filter pills report their state', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    expect(screen.getByLabelText('Choose a PC')).toHaveClass('pc-chip');
    expect(screen.getByLabelText('Choose a PC')).toHaveTextContent('coffee');
    const hidden = screen.getByRole('button', { name: 'Hidden' });
    expect(hidden).toHaveAttribute('aria-pressed', 'false');
    hidden.click();
    await waitFor(() => expect(hidden).toHaveAttribute('aria-pressed', 'true'));
  });

  it('lists dirs before files, each group alphabetical', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    const names = screen.getAllByTestId('files-row').map((row) => row.querySelector('.files-row-name')?.textContent);
    expect(names).toEqual(['src', 'zdir', 'a.txt', 'b.txt']);
  });

  it('shows a git badge for a changed file', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    expect(screen.getByText('M')).toBeTruthy();
  });

  it('Delete… from the row menu confirms and sends fs_delete (UNI-184)', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Actions for a.txt').click();
    screen.getByRole('menuitem', { name: /Delete/ }).click();
    const sheet = await screen.findByTestId('files-delete-sheet');
    expect(sheet.textContent).toContain('a.txt');
    [...sheet.querySelectorAll('button')].find((b) => b.textContent === 'Delete')!.click();
    await waitFor(() =>
      expect(vi.mocked(call)).toHaveBeenCalledWith('fs_delete', expect.objectContaining({ path: expect.stringMatching(/a\.txt$/), confirm: true }), expect.anything()),
    );
  });

  it('Rename… from the row menu opens the rename sheet (UNI-184)', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Actions for b.txt').click();
    screen.getByRole('menuitem', { name: /Rename/ }).click();
    const sheet = await screen.findByTestId('files-rename-sheet');
    expect(sheet.querySelector<HTMLInputElement>('input')!.value).toBe('b.txt');
  });

  it('Rename: Enter in the field submits once (fs_rename), even when pressed twice', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Actions for b.txt').click();
    screen.getByRole('menuitem', { name: /Rename/ }).click();
    const sheet = await screen.findByTestId('files-rename-sheet');
    const input = sheet.querySelector<HTMLInputElement>('input')!;
    fireEvent.input(input, { target: { value: 'c.txt' } });
    const form = input.closest('form')!;
    fireEvent.submit(form);
    fireEvent.submit(form);
    await waitFor(() => expect(vi.mocked(call)).toHaveBeenCalledWith('fs_rename', expect.objectContaining({ to: expect.stringMatching(/c\.txt$/) }), expect.anything()));
    expect(vi.mocked(call).mock.calls.filter(([c]) => c === 'fs_rename')).toHaveLength(1);
  });

  it('"Edit as text" from the row menu opens the editor with asText (UNI-191)', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Actions for a.txt').click();
    screen.getByRole('menuitem', { name: 'Edit as text' }).click();
    await waitFor(() => expect(location.hash).toBe('#/files/edit'));
    expect(getEditorTarget()).toMatchObject({ hostId: 'host-1', path: expect.stringMatching(/a\.txt$/), asText: true });
    clearEditorTarget();
  });

  it('New file: creates the file via fs_write(create:true) then opens the editor (UNI-198)', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Files menu').click();
    screen.getByRole('menuitem', { name: /New file/ }).click();
    const sheet = await screen.findByTestId('files-new-file-sheet');
    const input = sheet.querySelector<HTMLInputElement>('input')!;
    input.value = 'notes.md';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    [...sheet.querySelectorAll('button')].find((b) => b.textContent === 'Create')!.click();
    await waitFor(() =>
      expect(vi.mocked(call)).toHaveBeenCalledWith(
        'fs_write',
        expect.objectContaining({ path: expect.stringMatching(/notes\.md$/), create: true }),
        expect.anything(),
      ),
    );
    await waitFor(() => expect(location.hash).toBe('#/files/edit'));
    expect(getEditorTarget()).toMatchObject({ hostId: 'host-1', path: expect.stringMatching(/notes\.md$/) });
    clearEditorTarget();
  });

  describe('Mention in chat (UNI-193)', () => {
    it('asks which chat when several exist, queues the mention and opens the picked chat', async () => {
      setChatTransport({
        list: async () => chatListing([{ id: 'elsewhere', cwd: '/home/u/other' }, { id: 'here', cwd: '/home/u/proj' }]),
        open: () => ({ send: async () => {}, close: () => {} }),
      } as never);
      mount();
      await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
      screen.getByLabelText('Actions for a.txt').click();
      screen.getByRole('menuitem', { name: 'Mention in chat' }).click();
      await waitFor(() => expect(screen.getAllByTestId('chat-pick-row').length).toBe(2));
      expect(screen.getByText(/Mention a\.txt in/)).toBeTruthy();
      screen.getAllByTestId('chat-pick-row')[0]!.click();
      await waitFor(() => expect(consumeMentionFor('host-1:here')).toMatch(/^@.*a\.txt /));
    });

    it('a single candidate chat auto-picks (no sheet)', async () => {
      setChatTransport({ list: async () => chatListing([{ id: 'only', cwd: '/home/u/proj' }]), open: () => ({ send: async () => {}, close: () => {} }) } as never);
      mount();
      await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
      screen.getByLabelText('Actions for a.txt').click();
      screen.getByRole('menuitem', { name: 'Mention in chat' }).click();
      await waitFor(() => expect(consumeMentionFor('host-1:only')).toMatch(/^@.*a\.txt /));
      await waitFor(() => expect(screen.queryByTestId('chat-pick-sheet')).toBeNull());
    });
  });
});

describe('Files media tap (UNI-196)', () => {
  it('tapping a symlinked image opens that image, not the folder\'s first file', async () => {
    const { openPreviewer } = await import('../lib/media/previewerState');
    const spy = vi.spyOn(await import('../lib/media/previewerState'), 'openPreviewer');
    void openPreviewer;
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u/DBX'] } };
      if (cmd === 'fs_list')
        return {
          ok: true,
          value: {
            entries: [
              { name: 'DBX.png', kind: 'file', size: 10, mtime: 1, ignored: false },
              { name: 'dbx.png', kind: 'symlink', size: 10, mtime: 1, ignored: false },
              { name: 'AppRun', kind: 'file', size: 10, mtime: 1, ignored: false },
            ],
            more: false,
          },
        };
      return { ok: true, value: null };
    });
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(3));
    const row = screen.getAllByTestId('files-row').find((r) => r.querySelector('.files-row-name')?.textContent === 'dbx.png')!;
    row.click();
    await waitFor(() => expect(spy).toHaveBeenCalled());
    const req = spy.mock.calls.at(-1)![0] as { paths: string[]; index: number };
    expect(req.paths[req.index]).toMatch(/\/dbx\.png$/);
  });
});

function pngFile(name = 'x.png'): File {
  return new File([new Uint8Array([1, 2, 3])], name, { type: 'image/png' });
}

function setInputFiles(input: HTMLInputElement, files: File[]) {
  const list: Pick<FileList, 'length' | 'item'> & Iterable<File> & Record<number, File> = {
    length: files.length,
    item: (i: number) => files[i] ?? null,
    [Symbol.iterator]: () => files[Symbol.iterator](),
  };
  files.forEach((f, i) => (list[i] = f));
  Object.defineProperty(input, 'files', { value: list, configurable: true });
}

describe('Files upload (UNI-199)', () => {
  afterEach(() => setBlobTransport(undefined));

  it('uploads to the default shared folder: fs_import to ~/.unipi/uploads/<name>', async () => {
    const putBytes = vi.fn(async () => ({ sha256: 'a'.repeat(64), size: 3, deduped: false, path: '/blobs/a' }));
    setBlobTransport({ putBytes } as unknown as BlobTransport);
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u/proj'] } };
      if (cmd === 'fs_list') return { ok: true, value: listing };
      if (cmd === 'fs_import') return { ok: true, value: null };
      return { ok: true, value: null };
    });
    const { container } = mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Files menu').click();
    screen.getByRole('menuitem', { name: /Upload/ }).click();
    const sheet = await screen.findByTestId('files-upload-sheet');
    [...sheet.querySelectorAll('button')].find((b) => b.textContent?.startsWith('Choose files'))!.click();
    const input = container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile('x.png')]);
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => expect(putBytes).toHaveBeenCalled());
    await waitFor(() =>
      expect(vi.mocked(call)).toHaveBeenCalledWith(
        'fs_import',
        expect.objectContaining({ to: '~/.unipi/uploads/x.png', sha256: 'a'.repeat(64) }),
        expect.anything(),
      ),
    );
  });

  it('renames on a name clash: retries fs_import with "(1)" appended before the extension', async () => {
    const putBytes = vi.fn(async () => ({ sha256: 'b'.repeat(64), size: 3, deduped: false, path: '/blobs/b' }));
    setBlobTransport({ putBytes } as unknown as BlobTransport);
    const importCalls: string[] = [];
    vi.mocked(call).mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u/proj'] } };
      if (cmd === 'fs_list') return { ok: true, value: listing };
      if (cmd === 'fs_import') {
        const to = (args?.to as string) ?? '';
        importCalls.push(to);
        if (to === '~/.unipi/uploads/x.png') {
          return { ok: false, error: { code: 'E_CONFLICT', message: 'exists', retryable: false } };
        }
        return { ok: true, value: null };
      }
      return { ok: true, value: null };
    });
    const { container } = mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Files menu').click();
    screen.getByRole('menuitem', { name: /Upload/ }).click();
    const sheet = await screen.findByTestId('files-upload-sheet');
    [...sheet.querySelectorAll('button')].find((b) => b.textContent?.startsWith('Choose files'))!.click();
    const input = container.querySelector('input[type=file]') as HTMLInputElement;
    setInputFiles(input, [pngFile('x.png')]);
    input.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => expect(importCalls).toContain('~/.unipi/uploads/x (1).png'));
  });
});


describe('Upload destination (UNI-199 rework)', () => {
  it('the picked destination is the checked one', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    screen.getByLabelText('Files menu').click();
    screen.getByRole('menuitem', { name: /Upload/ }).click();
    const sheet = await screen.findByTestId('files-upload-sheet');
    const radios = () => [...sheet.querySelectorAll<HTMLElement>('[role="radio"]')];
    expect(radios()[0]!.getAttribute('aria-checked')).toBe('true');
    expect(radios()[1]!.getAttribute('aria-checked')).toBe('false');
    radios()[1]!.click();
    await waitFor(() => expect(radios()[1]!.getAttribute('aria-checked')).toBe('true'));
    expect(radios()[0]!.getAttribute('aria-checked')).toBe('false');
    expect(sheet.textContent).toContain('this folder');
  });
});

describe('Files → Git branch chip (UNI-217)', () => {
  afterEach(() => closeGit());
  it('a folder in a repo shows the branch chip (with ahead/behind + dirty dot); tapping opens Git there', async () => {
    const base = vi.mocked(call).getMockImplementation()!;
    vi.mocked(call).mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'git_op') return { ok: true, value: { repo: true, root: '/home/u/proj', branch: 'feat/x', ahead: 1, behind: 0, detached: false, remotes: ['origin'], stashes: 0 } } as never;
      return base(cmd, args, z.unknown());
    });
    mount();
    const chip = await screen.findByTestId('files-git-chip');
    expect(chip).toHaveTextContent('feat/x');
    expect(chip).toHaveTextContent('↑1');
    await waitFor(() => expect(chip.querySelector('.git-chip-dirty')).not.toBeNull()); // a.txt is M
    fireEvent.click(chip);
    expect(gitTarget()).toEqual({ hostId: 'host-1', path: '/home/u/proj', from: 'files' });
  });

  it('no chip outside a repo or on an older host', async () => {
    const base = vi.mocked(call).getMockImplementation()!;
    vi.mocked(call).mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'git_op') return { ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `git`', retryable: false } } as never;
      return base(cmd, args, z.unknown());
    });
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId('files-git-chip')).toBeNull();
  });
});

describe('desktop Files (UNI-117)', () => {
  const desktop = () => {
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} }));
    resetLayout();
  };
  beforeEach(desktop);
  afterEach(() => {
    setPlatformOverride(undefined);
    Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true });
    vi.unstubAllGlobals();
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    resetLayout();
    setBlobTransport(undefined);
  });
  const press = (key: string) => window.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));

  it('arrows select, F2 renames, Del asks, Enter opens a folder, Backspace goes up', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    press('ArrowDown');
    press('ArrowDown');
    press('ArrowDown');
    const rows = () => screen.getAllByTestId('files-row');
    await waitFor(() => expect(rows()[2]!.hasAttribute('data-selected')).toBe(true));
    press('F2');
    const rename = await screen.findByTestId('files-rename-sheet');
    expect(rename.querySelector<HTMLInputElement>('input')!.value).toBe('a.txt');
    // The name field gets the first focus (not the sheet's ✕), so typing renames.
    await waitFor(() => expect(document.activeElement).toBe(rename.querySelector('input')));
    rename.querySelector<HTMLButtonElement>('[aria-label=Close]')!.click();
    await waitFor(() => expect(screen.queryByTestId('files-rename-sheet')).toBeNull());
    press('Delete');
    const del = await screen.findByTestId('files-delete-sheet');
    expect(del.textContent).toContain('a.txt');
    expect(vi.mocked(call)).not.toHaveBeenCalledWith('fs_delete', expect.anything(), expect.anything());
    del.querySelector<HTMLButtonElement>('[aria-label=Close]')!.click();
    await waitFor(() => expect(screen.queryByTestId('files-delete-sheet')).toBeNull());
    press('Home');
    press('Enter');
    await waitFor(() => expect(vi.mocked(call)).toHaveBeenCalledWith('fs_list', expect.objectContaining({ path: '/home/u/proj/src' }), expect.anything()));
    press('Backspace');
    await waitFor(() => expect(vi.mocked(call).mock.calls.filter(([c, a]) => c === 'fs_list' && (a as { path: string }).path === '/home/u/proj').length).toBeGreaterThan(1));
  });

  it('root picker (UNI-220): two short roots are inline pills; opens one, marks it current', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u', '/tmp'] } };
      if (cmd === 'fs_list') return { ok: true, value: listing };
      return { ok: true, value: null };
    });
    mount();
    const places = await screen.findByRole('group', { name: 'Places' });
    const pills = Array.from(places.querySelectorAll('button'));
    expect(pills.map((b) => b.textContent)).toEqual(['~', '/tmp']);
    // The Places live in the same toolbar as Hidden / Changed (UNI-218), not a strip of their own.
    expect(places.closest('[role=toolbar]')).not.toBeNull();
    await waitFor(() => expect(pills[0]!.getAttribute('aria-current')).toBe('true'));
    fireEvent.click(pills[1]!);
    await waitFor(() =>
      expect(vi.mocked(call)).toHaveBeenCalledWith('fs_list', expect.objectContaining({ path: '/tmp', outside: undefined }), expect.anything()),
    );
    await waitFor(() => expect(pills[1]!.getAttribute('aria-current')).toBe('true'));
    expect(pills[0]!.getAttribute('aria-current')).toBeNull();
  });

  it('Places are per-host: a PC whose fs_roots fails keeps the previous PC\'s roots out (Windows $HOME case)', async () => {
    const twoHosts = [
      { id: 'host-1', name: 'oi', ticket: 't', paired_at: 0, last_path: 'lan' as const },
      { id: 'host-2', name: 'COFFEE', ticket: 't', paired_at: 0, last_path: 'lan' as const },
    ];
    vi.mocked(call).mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'list_hosts') return { ok: true, value: twoHosts };
      if (cmd === 'fs_roots') {
        const hostId = (args as { hostId?: string } | undefined)?.hostId;
        // The Linux PC answers with its roots; the Windows host fails fs_roots
        // (that is what a host with no $HOME did before the USERPROFILE fix).
        if (hostId === 'host-2') {
          return { ok: false, error: { code: 'E_INTERNAL', message: '$HOME is not set on the host', retryable: false } };
        }
        return { ok: true, value: { roots: ['/home/u', '/tmp'] } };
      }
      if (cmd === 'fs_list') return { ok: true, value: listing };
      return { ok: true, value: null };
    });
    mount();
    // Host 1's Places are shown.
    const places = await screen.findByRole('group', { name: 'Places' });
    await waitFor(() => expect(Array.from(places.querySelectorAll('button')).map((b) => b.textContent)).toEqual(['~', '/tmp']));

    // Switch to the failing PC.
    fireEvent.click(screen.getByLabelText('Choose a PC'));
    fireEvent.click(await screen.findByRole('button', { name: /COFFEE/ }));

    // The failure is explained instead of an empty folder…
    expect(await screen.findByText('$HOME is not set on the host')).toBeInTheDocument();
    // …and its Places are empty: not the other PC's /home/… and /tmp.
    const after = screen.queryByRole('group', { name: 'Places' });
    const labels = after ? Array.from(after.querySelectorAll('button')).map((b) => b.textContent) : [];
    expect(labels).not.toContain('/tmp');
    expect(labels.some((l) => (l ?? '').includes('~'))).toBe(false);
  });

  it('many roots collapse into one Places menu pill (UNI-218); the breadcrumb starts at the root, not /', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u', '/tmp', '/mnt', '/home/u/proj'] } };
      if (cmd === 'fs_list') return { ok: true, value: listing };
      return { ok: true, value: null };
    });
    mount();
    const places = await screen.findByRole('group', { name: 'Places' });
    const pill = places.querySelector<HTMLButtonElement>('.files-places-menu')!;
    expect(places.querySelectorAll('button').length).toBe(1);
    await waitFor(() => expect(pill.textContent).toBe('~'));
    const crumbs = () => Array.from(screen.queryByRole('navigation', { name: 'Breadcrumb' })?.querySelectorAll('button') ?? []).map((b) => b.textContent);
    // At a root the Places pill names it: no one-crumb breadcrumb row.
    await waitFor(() => expect(screen.queryByRole('navigation', { name: 'Breadcrumb' })).toBeNull());
    fireEvent.click(pill);
    const menu = await screen.findByRole('menu', { name: 'Places' });
    const rows = Array.from(menu.querySelectorAll('[role=menuitem]'));
    expect(rows.map((r) => r.querySelector('.truncate')?.textContent)).toEqual(['~', '/tmp', '/mnt', 'proj']);
    fireEvent.click(rows[1]!);
    await waitFor(() => expect(vi.mocked(call)).toHaveBeenCalledWith('fs_list', expect.objectContaining({ path: '/tmp' }), expect.anything()));
    await waitFor(() => expect(pill.textContent).toBe('/tmp'));
    expect(crumbs()).toEqual([]);
    await waitFor(() => expect(screen.queryByRole('menu', { name: 'Places' })).toBeNull());
  });

  it('offline PC (UNI-218 F56): the shared "Can\'t reach" state with Retry, not an empty folder', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots' || cmd === 'fs_list') return { ok: false, error: { code: 'E_TRANSPORT', message: 'Could not reach coffee', hint: 'Is unipi-host running on the PC?', retryable: true } };
      return { ok: true, value: null };
    });
    mount();
    const state = await screen.findByTestId('offline-state');
    expect(state).toHaveTextContent("Can't reach coffee");
    expect(state).toHaveTextContent('Is unipi-host running on the PC?');
    expect(screen.queryByText('This folder is empty')).toBeNull();
    expect(screen.queryByRole('toolbar', { name: 'Filters' })).toBeNull();
    const before = vi.mocked(call).mock.calls.filter(([c]) => c === 'fs_roots').length;
    fireEvent.click(screen.getByRole('button', { name: /Retry/ }));
    await waitFor(() => expect(vi.mocked(call).mock.calls.filter(([c]) => c === 'fs_roots').length).toBeGreaterThan(before));
  });

  it('opened at a session cwd outside $HOME ("Files here"): Backspace stops at that root, never lists its parent', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u', '/tmp/work'] } };
      if (cmd === 'fs_list') return { ok: true, value: listing };
      return { ok: true, value: null };
    });
    setFilesTarget({ hostId: 'host-1', path: '/tmp/work' });
    mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    await waitFor(() => expect(vi.mocked(call)).toHaveBeenCalledWith('fs_roots', expect.anything(), expect.anything()));
    await new Promise((r) => setTimeout(r, 0));
    press('Backspace');
    await new Promise((r) => setTimeout(r, 20));
    const listed = vi.mocked(call).mock.calls.filter(([c]) => c === 'fs_list').map(([, a]) => (a as { path: string }).path);
    expect(listed).not.toContain('/tmp');
  });

  it('files dropped from the OS upload into the folder you are in', async () => {
    const putBytes = vi.fn(async () => ({ sha256: 'c'.repeat(64), size: 3, deduped: false, path: '/blobs/c' }));
    setBlobTransport({ putBytes } as unknown as BlobTransport);
    const { container } = mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    const screenEl = container.querySelector('.files-screen')!;
    const file = pngFile('drop.png');
    const dt = { types: ['Files'], files: [file], dropEffect: 'none' };
    const ev = (type: string) => Object.assign(new Event(type, { bubbles: true, cancelable: true }), { dataTransfer: dt });
    screenEl.dispatchEvent(ev('dragenter'));
    expect(await screen.findByTestId('files-drop')).toBeTruthy();
    screenEl.dispatchEvent(ev('drop'));
    await waitFor(() => expect(screen.queryByTestId('files-drop')).toBeNull());
    await waitFor(() => expect(vi.mocked(call)).toHaveBeenCalledWith('fs_import', expect.objectContaining({ to: '/home/u/proj/drop.png' }), expect.anything()));
  });

  it('a text drag (not files) is ignored', async () => {
    const { container } = mount();
    await waitFor(() => expect(screen.getAllByTestId('files-row').length).toBe(4));
    const e = Object.assign(new Event('dragenter', { bubbles: true, cancelable: true }), { dataTransfer: { types: ['text/plain'] } });
    container.querySelector('.files-screen')!.dispatchEvent(e);
    expect(screen.queryByTestId('files-drop')).toBeNull();
  });
});
