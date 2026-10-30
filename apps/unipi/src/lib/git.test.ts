import { describe, expect, it, vi, afterEach } from 'vitest';
import {
  aheadBehind, branchLabel, canCommit, changeKey, closeGit, commitLabel, flattenGroups, git, gitErrorText, gitTarget,
  GIT_SOURCES, groupChanges, isClean, isCommitKey, lastTurnFiles, openGit, refPills, relativeTime, sourceGroups, splitPath,
  statusWord, subjectOf, totals, type GitFileChange, type GitSource,
} from './git';
import type { Turn } from './session/turns';
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


describe('review sources (WP-A15, Z-96)', () => {
  const f = (path: string, area: GitFileChange['area'], status = 'M'): GitFileChange => ({ path, area, status, binary: false });
  const files = [f('c.rs', 'conflict', 'U'), f('s.ts', 'staged'), f('a.ts', 'unstaged'), f('n.md', 'untracked', '?')];

  it('Unstaged keeps the unipi grouping minus Staged; Staged is the index only', () => {
    expect(sourceGroups('unstaged', files).map((g) => [g.key, g.files.map((x) => x.path)])).toEqual([
      ['conflict', ['c.rs']],
      ['unstaged', ['a.ts']],
      ['untracked', ['n.md']],
    ]);
    expect(sourceGroups('staged', files).map((g) => [g.key, g.files])).toEqual([['staged', [f('s.ts', 'staged')]]]);
  });

  it('Branch and Last turn are one sorted group with the given label', () => {
    expect(sourceGroups('branch', [f('b.ts', 'staged'), f('a.ts', 'staged')], 'vs origin/main')).toEqual([
      { key: 'branch', label: 'vs origin/main', files: [f('a.ts', 'staged'), f('b.ts', 'staged')] },
    ]);
    expect(sourceGroups('turn', [])).toEqual([]);
  });

  it('flatten interleaves group heads and file rows, and find filters by path', () => {
    const flat = flattenGroups(sourceGroups('unstaged', files));
    expect(flat.rows.map((r) => r.kind)).toEqual(['group', 'file', 'group', 'file', 'group', 'file']);
    expect(flat.matches).toBe(3);
    const found = flattenGroups(sourceGroups('unstaged', files), 'a.t');
    expect(found.matches).toBe(1);
    expect(found.rows[1]).toMatchObject({ kind: 'file', change: { path: 'a.ts' } });
  });

  it('lastTurnFiles takes the last turn\'s write/edit tool paths, made repo-relative, deduped', () => {
    const tool = (id: string, name: string, args: Record<string, unknown>) =>
      ({ id, name, args, state: 'ok' }) as never;
    const turn = (id: string, tools: Array<[string, string, Record<string, unknown>]>) =>
      ({
        turnId: id,
        kind: 'turn',
        live: false,
        stopped: false,
        source: 'user',
        entries: [],
        segments: [{ id: `${id}-s`, index: 0, work: tools.map(([tid, name, args], i) => ({ kind: 'tool', key: `${id}-${i}`, tool: tool(tid, name, args) })), nodes: [], live: false }],
      }) as Turn;
    const turns = [
      turn('t1', [['w1', 'write', { path: 'old.txt', content: 'x' }]]),
      turn('t2', [
        ['w2', 'edit', { file_path: '/home/u/proj/src/a.ts', oldText: 'x', newText: 'y' }],
        ['w3', 'write', { path: '/home/u/proj/src/a.ts', content: 'z' }],
        ['r1', 'read', { path: '/home/u/proj/other.txt' }],
        ['w4', 'write', { path: '/etc/passwd', content: 'no' }],
        ['b1', 'bash', { command: 'true' }],
        ['w5', 'edit', { path: 'src/b.ts', oldText: 'a', newText: 'b' }],
      ]),
    ];
    expect(lastTurnFiles(turns, '/home/u/proj')).toEqual(['src/a.ts', 'src/b.ts']);
    expect(lastTurnFiles([])).toEqual([]);
    expect(lastTurnFiles([{ ...turn('t3', []), kind: 'preamble' } as Turn])).toEqual([]);
  });

  it('builds the source wire commands (turn_files, branch_files, git_watch)', async () => {
    await git.turnFiles('h', '/r', ['a.ts'], { staged: true });
    await git.branchFiles('h', '/r');
    await git.branchFiles('h', '/r', 'main');
    await git.watch('h', '/r', false);
    const cmds = vi.mocked(call).mock.calls.filter(([op]) => op === 'git_op' || op === 'git_watch').map((c) => [c[0], c[1]]);
    expect(cmds).toEqual([
      ['git_op', { hostId: 'h', path: '/r', outside: undefined, cmd: { action: 'turn_files', files: ['a.ts'], staged: true } }],
      ['git_op', { hostId: 'h', path: '/r', outside: undefined, cmd: { action: 'branch_files' } }],
      ['git_op', { hostId: 'h', path: '/r', outside: undefined, cmd: { action: 'branch_files', base: 'main' } }],
      ['git_watch', { hostId: 'h', root: '/r', on: false }],
    ]);
  });

  it('GIT_SOURCES covers the four ZCode sources', () => {
    const ids: GitSource[] = ['unstaged', 'staged', 'branch', 'turn'];
    expect(GIT_SOURCES.map((s) => s.id)).toEqual(ids);
  });
});
