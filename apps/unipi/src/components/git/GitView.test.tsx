// @vitest-environment jsdom
import { render, screen, waitFor, cleanup, fireEvent, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import { GitView } from './GitView';
import { call } from '../../lib/ipc';
import { closeGit, openGit } from '../../lib/git';
import { setPlatformOverride } from '../../lib/platform';
import { resetLayout } from '../../lib/layout';
import { getEditorTarget, clearEditorTarget } from '../../lib/editorTarget';

vi.mock('../../lib/ipc', () => ({ call: vi.fn() }));

type Cmd = { action: string; [k: string]: unknown };
type Reply = { ok: true; value: unknown } | { ok: false; error: { code: string; message: string; hint?: string; retryable: boolean } };

const repo = { repo: true, root: '/home/u/proj', branch: 'main', head: 'abc1234', detached: false, upstream: 'origin/main', ahead: 2, behind: 1, remotes: ['origin'], stashes: 1 };
let files: Array<Record<string, unknown>>;
let calls: Cmd[];
let overrides: Partial<Record<string, (cmd: Cmd) => Reply>>;

function reply(cmd: Cmd): Reply {
  const o = overrides[cmd.action];
  if (o) return o(cmd);
  switch (cmd.action) {
    case 'repo': return { ok: true, value: repo };
    case 'status': return { ok: true, value: { ...repo, files, more: false } };
    case 'diff': return { ok: true, value: { text: `diff --git a/${cmd.file} b/${cmd.file}\n--- a/${cmd.file}\n+++ b/${cmd.file}\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n`, truncated: false, binary: false } };
    case 'stage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path as string);
      files = files.map((f) => (paths.includes(f.path as string) ? { ...f, area: 'staged' } : f));
      return { ok: true, value: null };
    }
    case 'unstage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path as string);
      files = files.map((f) => (paths.includes(f.path as string) && f.area === 'staged' ? { ...f, area: f.status === 'A' ? 'untracked' : 'unstaged' } : f));
      return { ok: true, value: null };
    }
    case 'commit':
      files = files.filter((f) => f.area !== 'staged');
      return { ok: true, value: { hash: 'def5678', subject: String(cmd.message).split('\n')[0] } };
    case 'discard':
      files = files.filter((f) => !(cmd.paths as string[]).includes(f.path as string));
      return { ok: true, value: null };
    case 'log': {
      const all = Array.from({ length: 60 }, (_, i) => ({ hash: `h${i}`.padEnd(40, '0'), short: `h${i}`, subject: `commit ${i}`, author: 'Ann', time: 1_700_000_000 - i * 3600, refs: i === 0 ? ['HEAD -> main', 'origin/main'] : [] }));
      const skip = Number(cmd.skip ?? 0);
      const limit = Number(cmd.limit ?? 50);
      return { ok: true, value: { commits: all.slice(skip, skip + limit), more: skip + limit < all.length } };
    }
    case 'show':
      return { ok: true, value: { commit: { hash: String(cmd.rev), short: String(cmd.rev).slice(0, 7), subject: 'commit 0', author: 'Ann', time: 1_700_000_000, refs: [] }, body: 'Longer body.', files: [{ path: 'a.txt', status: 'M', add: 1, del: 1 }], diff: 'diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\n', truncated: false } };
    case 'branches':
      return { ok: true, value: { branches: [{ name: 'main', current: true, upstream: 'origin/main', ahead: 2, behind: 1, time: 1_700_000_000, subject: 'x' }, { name: 'feat/x', time: 1_699_000_000, subject: 'y' }, { name: 'origin/main', remote: true }] } };
    case 'switch': return { ok: true, value: null };
    case 'push': return { ok: true, value: { output: 'To origin\n   abc..def  main -> main' } };
    case 'pull': return { ok: true, value: { output: 'Already up to date.' } };
    default: return { ok: true, value: { output: 'Done.' } };
  }
}

beforeEach(() => {
  files = [
    { path: 'src/a.ts', area: 'staged', status: 'M', add: 3, del: 1, binary: false },
    { path: 'b.txt', area: 'unstaged', status: 'M', add: 1, del: 1, binary: false },
    { path: 'new.md', area: 'untracked', status: '?', add: 12, binary: false },
  ];
  calls = [];
  overrides = {};
  vi.mocked(call).mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd !== 'git_op') return { ok: true, value: null } as never;
    const c = args!.cmd as Cmd;
    calls.push(c);
    return reply(c) as never;
  });
});

afterEach(() => {
  closeGit();
  cleanup();
  vi.resetAllMocks();
  clearEditorTarget();
  location.hash = '#/';
});

function mount(path = '/home/u/proj/src') {
  openGit({ hostId: 'h1', path, from: 'files' });
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <GitView />} />
    </HashRouter>
  ));
}

describe('Git view (UNI-217) — phone', () => {
  it('lists Staged / Changes / Untracked with +/- counts and the branch line', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    expect(screen.getByText('proj')).toBeInTheDocument();
    expect(screen.getByText('main · origin/main ↑2 ↓1')).toBeInTheDocument();
    const sections = screen.getAllByRole('region').map((s) => s.getAttribute('aria-label'));
    expect(sections).toEqual(['Staged', 'Changes', 'Untracked']);
    const staged = screen.getByRole('region', { name: 'Staged' });
    expect(within(staged).getAllByText('+3')[0]).toHaveClass('add');
    expect(within(staged).getAllByText('−1')[0]).toHaveClass('del');
    expect(screen.getByLabelText('Pull 1 commits')).toBeInTheDocument();
    expect(screen.getByLabelText('Push 2 commits')).toBeInTheDocument();
  });

  it('stage toggle sends stage/unstage and moves the row', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('Stage b.txt'));
    await waitFor(() => expect(within(screen.getByRole('region', { name: 'Staged' })).getAllByTestId('git-row').length).toBe(2));
    expect(calls.find((c) => c.action === 'stage')).toEqual({ action: 'stage', paths: ['b.txt'] });
    fireEvent.click(screen.getByLabelText('Unstage src/a.ts'));
    await waitFor(() => expect(calls.some((c) => c.action === 'unstage')).toBe(true));
    fireEvent.click(screen.getByRole('button', { name: 'Stage all' }));
    await waitFor(() => expect(calls.filter((c) => c.action === 'stage').length).toBe(2));
  });

  it('commit composer: disabled until a message, commits, clears, toasts', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    const button = screen.getByRole('button', { name: /Commit 1 file/ });
    expect(button).toBeDisabled();
    const box = screen.getByLabelText('Commit message') as HTMLTextAreaElement;
    fireEvent.input(box, { target: { value: 'feat: thing\n\nbody' } });
    expect(button).not.toBeDisabled();
    fireEvent.click(button);
    await waitFor(() => expect(screen.getByTestId('git-toast')).toHaveTextContent('Committed def5678 · feat: thing'));
    expect(calls.find((c) => c.action === 'commit')).toEqual({ action: 'commit', message: 'feat: thing\n\nbody', amend: false });
    expect(box.value).toBe('');
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Staged' })).toBeNull());
  });

  it('Ctrl+Enter commits from the message box', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    const box = screen.getByLabelText('Commit message');
    fireEvent.input(box, { target: { value: 'quick' } });
    fireEvent.keyDown(box, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(calls.some((c) => c.action === 'commit')).toBe(true));
  });

  it('tap a file → diff layer with the shared diff view; Back closes it', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('b.txt, modified'));
    const layer = await screen.findByTestId('git-detail-layer');
    await waitFor(() => expect(layer.querySelector('.diffview .diff-add')).not.toBeNull());
    expect(calls.find((c) => c.action === 'diff')).toEqual({ action: 'diff', file: 'b.txt', staged: false, untracked: false });
    fireEvent.click(within(layer).getByLabelText('Close diff'));
    await waitFor(() => expect(screen.queryByTestId('git-detail-layer')).toBeNull());
  });

  it('discard asks first, then sends discard with confirm', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('b.txt, modified'));
    const layer = await screen.findByTestId('git-detail-layer');
    fireEvent.click(within(layer).getByLabelText('Discard changes'));
    const sheet = await screen.findByTestId('git-discard-sheet');
    expect(sheet).toHaveTextContent("can't be undone");
    expect(calls.some((c) => c.action === 'discard')).toBe(false);
    fireEvent.click(within(sheet).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(calls.find((c) => c.action === 'discard')).toEqual({ action: 'discard', paths: ['b.txt'], confirm: true }));
    await waitFor(() => expect(screen.queryByTestId('git-detail-layer')).toBeNull());
  });

  it('push failure shows git\'s message and the auth hint plainly', async () => {
    overrides.push = () => ({ ok: false, error: { code: 'E_GIT', message: "fatal: could not read Username for 'https://github.com': terminal prompts disabled", hint: 'Auth needed: this PC has no credentials…', retryable: false } });
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('Push 2 commits'));
    const banner = await screen.findByTestId('git-action-error');
    expect(banner).toHaveTextContent('could not read Username');
    expect(banner).toHaveTextContent('Auth needed');
    fireEvent.click(within(banner).getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(screen.queryByTestId('git-action-error')).toBeNull());
  });

  it('pull shows progress, then a toast', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    vi.mocked(call).mockImplementation(async (_cmd: string, args?: Record<string, unknown>) => {
      const c = args!.cmd as Cmd;
      calls.push(c);
      if (c.action === 'pull') await gate;
      return reply(c) as never;
    });
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('Pull 1 commits'));
    expect(await screen.findByTestId('git-progress')).toHaveTextContent('Pulling…');
    expect(screen.getByLabelText('Push 2 commits')).toBeDisabled();
    release();
    await waitFor(() => expect(screen.getByTestId('git-toast')).toHaveTextContent('Pulled · Already up to date.'));
  });

  it('history: pages of commits, load more, open a commit', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByRole('tab', { name: 'History' }));
    await waitFor(() => expect(screen.getAllByTestId('git-commit-row').length).toBe(50));
    expect(screen.getByText('main', { selector: '.git-ref' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Load more' }));
    await waitFor(() => expect(screen.getAllByTestId('git-commit-row').length).toBe(60));
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
    fireEvent.click(screen.getAllByTestId('git-commit-row')[0]!);
    const commit = await screen.findByTestId('git-commit');
    await waitFor(() => expect(commit).toHaveTextContent('Longer body.'));
    expect(commit.querySelector('.diffview')).not.toBeNull();
  });

  it('branch sheet lists local and remote; create and switch', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.click(screen.getByLabelText('Git menu'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Switch branch…' }));
    const sheet = await screen.findByTestId('git-branch-sheet');
    await waitFor(() => expect(within(sheet).getAllByTestId('git-branch-row').length).toBe(3));
    expect(within(sheet).getAllByTestId('git-branch-row')[0]).toBeDisabled(); // current
    fireEvent.click(within(sheet).getByText('feat/x'));
    await waitFor(() => expect(calls.find((c) => c.action === 'switch')).toEqual({ action: 'switch', branch: 'feat/x', create: false }));
    await waitFor(() => expect(screen.getByTestId('git-toast')).toHaveTextContent('Switched to feat/x'));
    fireEvent.click(screen.getByLabelText('Git menu'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Switch branch…' }));
    const sheet2 = await screen.findByTestId('git-branch-sheet');
    fireEvent.input(within(sheet2).getByLabelText('New branch name'), { target: { value: 'fix/y' } });
    fireEvent.click(within(sheet2).getByRole('button', { name: /Create/ }));
    await waitFor(() => expect(calls.filter((c) => c.action === 'switch').at(-1)).toEqual({ action: 'switch', branch: 'fix/y', create: true }));
  });

  it('not a repo → EmptyState; older host → update hint', async () => {
    overrides.repo = () => ({ ok: true, value: { repo: false } });
    mount('/home/u/notes');
    expect(await screen.findByTestId('git-not-repo')).toHaveTextContent('Not a git repository');
    closeGit();
    cleanup();
    overrides.repo = () => ({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `git`, expected one of `list`', retryable: false } });
    mount();
    expect(await screen.findByTestId('git-load-error')).toHaveTextContent('Update UniPi on the PC');
  });

  it('clean tree → "Nothing to commit"', async () => {
    files = [];
    mount();
    expect(await screen.findByTestId('git-clean')).toHaveTextContent('Nothing to commit');
  });

  it('row menu → Open in editor sets the editor target at the repo path', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    fireEvent.contextMenu(screen.getByLabelText('src/a.ts, modified'));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Open in editor' }));
    expect(getEditorTarget()).toEqual({ hostId: 'h1', path: '/home/u/proj/src/a.ts' });
  });
});

describe('Git view (UNI-217) — desktop', () => {
  beforeEach(() => {
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} }));
    resetLayout();
  });
  afterEach(() => {
    setPlatformOverride(undefined);
    Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true });
    vi.unstubAllGlobals();
    vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
    resetLayout();
  });
  const press = (key: string, extra: KeyboardEventInit = {}) => window.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...extra }));

  it('list | detail: placeholder, ↓ selects into the pane, Space stages', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    expect(screen.getByTestId('git-detail-placeholder')).toHaveTextContent('Pick a file');
    expect(screen.queryByTestId('git-detail-layer')).toBeNull();
    press('ArrowDown');
    await waitFor(() => expect(screen.getByTestId('git-diff')).toBeInTheDocument());
    expect(screen.getAllByTestId('git-row')[0]).toHaveAttribute('data-selected');
    press('ArrowDown');
    await waitFor(() => expect(screen.getAllByTestId('git-row')[1]).toHaveAttribute('data-selected'));
    press(' ');
    await waitFor(() => expect(calls.find((c) => c.action === 'stage')).toEqual({ action: 'stage', paths: ['b.txt'] }));
  });

  it('Ctrl+Enter anywhere commits when a message is ready', async () => {
    mount();
    await waitFor(() => expect(screen.getAllByTestId('git-row').length).toBe(3));
    press('Enter', { ctrlKey: true });
    expect(calls.some((c) => c.action === 'commit')).toBe(false);
    fireEvent.input(screen.getByLabelText('Commit message'), { target: { value: 'desk' } });
    press('Enter', { ctrlKey: true });
    await waitFor(() => expect(calls.some((c) => c.action === 'commit')).toBe(true));
  });
});
