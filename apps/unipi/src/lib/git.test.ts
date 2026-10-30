import { describe, expect, it, vi, afterEach } from 'vitest';
import {
  aheadBehind, branchLabel, canCommit, changeKey, closeGit, commitLabel, git, gitErrorText, gitTarget, groupChanges,
  isClean, isCommitKey, openGit, refPills, relativeTime, splitPath, statusWord, subjectOf, totals, type GitFileChange,
} from './git';
import { call } from './ipc';

vi.mock('./ipc', () => ({ call: vi.fn(async () => ({ ok: true, value: null })) }));
afterEach(() => vi.mocked(call).mockClear());

const f = (path: string, area: GitFileChange['area'], status = 'M', add?: number, del?: number, binary = false): GitFileChange => ({ path, area, status, add, del, binary });

describe('git helpers (UNI-217)', () => {
  it('groups Conflicts, Staged, Changes, Untracked; drops empty groups; sorts by path', () => {
    const groups = groupChanges([f('b.ts', 'unstaged'), f('z.ts', 'staged', 'A'), f('a.ts', 'unstaged'), f('n.md', 'untracked', '?'), f('c.rs', 'conflict', 'U')]);
    expect(groups.map((g) => g.label)).toEqual(['Conflicts', 'Staged', 'Changes', 'Untracked']);
    expect(groups[2]!.files.map((x) => x.path)).toEqual(['a.ts', 'b.ts']);
    expect(groupChanges([f('a', 'staged')]).map((g) => g.key)).toEqual(['staged']);
    expect(groupChanges([])).toEqual([]);
  });

  it('totals +/- and ignores binary files', () => {
    expect(totals([f('a', 'staged', 'M', 3, 1), f('b', 'staged', 'M', 2, 0), f('img', 'staged', 'M', undefined, undefined, true)])).toEqual({ add: 5, del: 1 });
  });

  it('keys a file per area (staged and unstaged copies differ)', () => {
    expect(changeKey(f('a', 'staged'))).not.toBe(changeKey(f('a', 'unstaged')));
  });

  it('splits paths and names statuses', () => {
    expect(splitPath('src/lib/a.ts')).toEqual({ name: 'a.ts', dir: 'src/lib' });
    expect(splitPath('a.ts')).toEqual({ name: 'a.ts', dir: '' });
    expect(statusWord('R')).toBe('renamed');
    expect(statusWord('?')).toBe('untracked');
    expect(statusWord('U')).toBe('conflict');
  });

  it('relative dates', () => {
    const now = 1_760_000_000_000;
    const at = (s: number) => relativeTime(now / 1000 - s, now);
    expect(at(10)).toBe('just now');
    expect(at(5 * 60)).toBe('5m ago');
    expect(at(3 * 3600)).toBe('3h ago');
    expect(at(2 * 86400)).toBe('2d ago');
    expect(at(21 * 86400)).toBe('3w ago');
    expect(at(90 * 86400)).toBe('3mo ago');
    expect(at(800 * 86400)).toBe('2y ago');
  });

  it('branch chip labels', () => {
    expect(aheadBehind({ ahead: 2, behind: 1 })).toBe('↑2 ↓1');
    expect(aheadBehind({ ahead: 0, behind: 3 })).toBe('↓3');
    expect(aheadBehind({ ahead: 0, behind: 0 })).toBe('');
    expect(branchLabel({ branch: 'main', detached: false })).toBe('main');
    expect(branchLabel({ head: 'abc1234', detached: true })).toBe('detached @abc1234');
    expect(branchLabel({ detached: true })).toBe('no branch');
  });

  it('ref pills', () => {
    expect(refPills(['HEAD -> main', 'origin/main', 'origin/HEAD', 'tag: v1.0', 'feat'])).toEqual([
      { label: 'main', kind: 'head' },
      { label: 'origin/main', kind: 'remote' },
      { label: 'v1.0', kind: 'tag' },
      { label: 'feat', kind: 'branch' },
    ]);
  });

  it('commit button label and enablement', () => {
    expect(commitLabel(0, false)).toBe('Commit');
    expect(commitLabel(1, false)).toBe('Commit 1 file');
    expect(commitLabel(3, false)).toBe('Commit 3 files');
    expect(commitLabel(0, true)).toBe('Amend message');
    expect(canCommit('fix', 1, false, false)).toBe(true);
    expect(canCommit('  ', 1, false, false)).toBe(false);
    expect(canCommit('fix', 0, false, false)).toBe(false);
    expect(canCommit('fix', 0, true, false)).toBe(true);
    expect(canCommit('fix', 1, false, true)).toBe(false);
    expect(subjectOf('\n  first line \nbody')).toBe('first line');
  });

  it('Ctrl/⌘+Enter is the commit key', () => {
    expect(isCommitKey({ key: 'Enter', ctrlKey: true, metaKey: false })).toBe(true);
    expect(isCommitKey({ key: 'Enter', ctrlKey: false, metaKey: true })).toBe(true);
    expect(isCommitKey({ key: 'Enter', ctrlKey: false, metaKey: false })).toBe(false);
    expect(isCommitKey({ key: 'Enter', ctrlKey: true, metaKey: false, altKey: true })).toBe(false);
  });

  it('error wording: an older host says update, others stay plain', () => {
    expect(gitErrorText({ code: 'E_SCHEMA', message: 'unknown variant `git`, expected one of `list`', retryable: false }).title).toBe('Update UniPi on the PC');
    const auth = gitErrorText({ code: 'E_GIT', message: "fatal: could not read Username for 'https://x'", hint: 'Auth needed: …', retryable: false });
    expect(auth.body).toContain('could not read Username');
    expect(auth.hint).toContain('Auth needed');
    expect(gitErrorText({ code: 'E_SCOPE', message: 'outside', retryable: false }).title).toBe('Folder not allowed');
  });

  it('isClean', () => {
    expect(isClean(undefined)).toBe(false);
    expect(isClean({ repo: true, detached: false, ahead: 0, behind: 0, remotes: [], stashes: 0, files: [], more: false })).toBe(true);
  });

  it('open/close target', () => {
    openGit({ hostId: 'h', path: '/p' });
    expect(gitTarget()).toEqual({ hostId: 'h', path: '/p' });
    closeGit();
    expect(gitTarget()).toBeUndefined();
  });

  it('builds the wire commands', async () => {
    await git.stage('h', '/r', 'all');
    await git.unstage('h', '/r', ['a.txt']);
    await git.discard('h', '/r', ['a.txt']);
    await git.commit('h', '/r', 'msg', true);
    await git.log('h', '/r', 50);
    await git.switch('h', '/r', 'feat', true);
    await git.pull('h', '/r');
    const cmds = vi.mocked(call).mock.calls.map((c) => [c[0], (c[1] as { cmd: unknown }).cmd]);
    expect(cmds).toEqual([
      ['git_op', { action: 'stage', all: true }],
      ['git_op', { action: 'unstage', paths: ['a.txt'] }],
      ['git_op', { action: 'discard', paths: ['a.txt'], confirm: true }],
      ['git_op', { action: 'commit', message: 'msg', amend: true }],
      ['git_op', { action: 'log', skip: 50, limit: 50 }],
      ['git_op', { action: 'switch', branch: 'feat', create: true }],
      ['git_op', { action: 'pull', rebase: false }],
    ]);
  });
});
