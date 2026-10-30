/**
 * Basic git (UNI-217): the UI's seam onto the `git_op` Tauri command
 * (`src-tauri/src/fs.rs` → host `{op:"git"}`, `crates/host/src/git.rs`),
 * plus the pure helpers the Git view uses (grouping, counts, relative
 * dates, ahead/behind labels, error wording). Tests: `git.test.ts`.
 */
import { createSignal } from 'solid-js';
import { z } from 'zod';
import {
  GitBranchesDataSchema,
  GitCommitDataSchema,
  GitDiffDataSchema,
  GitLogDataSchema,
  GitRepoDataSchema,
  GitShowDataSchema,
  GitStatusDataSchema,
  GitSyncDataSchema,
  GitShortstatDataSchema,
  type AppError,
  type GitCmd,
  type GitFileChange,
  type GitRepoData,
  type GitStatusData,
} from '@unipi/protocol';
import { call, type Result } from './ipc';

export type { GitFileChange, GitRepoData, GitStatusData, GitCommitInfo, GitBranch, GitShowData, GitDiffData } from '@unipi/protocol';

function op<T>(hostId: string, path: string, cmd: GitCmd, schema: z.ZodType<T>, outside?: boolean): Promise<Result<T>> {
  return call('git_op', { hostId, path, outside, cmd }, schema);
}

const Nothing = z.unknown();

export const git = {
  repo: (h: string, p: string) => op(h, p, { action: 'repo' }, GitRepoDataSchema),
  status: (h: string, p: string) => op(h, p, { action: 'status' }, GitStatusDataSchema),
  diff: (h: string, p: string, file: string, staged: boolean, untracked: boolean) =>
    op(h, p, { action: 'diff', file, staged, untracked }, GitDiffDataSchema),
  stage: (h: string, p: string, paths: string[] | 'all') =>
    op(h, p, paths === 'all' ? { action: 'stage', all: true } : { action: 'stage', paths }, Nothing),
  unstage: (h: string, p: string, paths: string[] | 'all') =>
    op(h, p, paths === 'all' ? { action: 'unstage', all: true } : { action: 'unstage', paths }, Nothing),
  discard: (h: string, p: string, paths: string[]) => op(h, p, { action: 'discard', paths, confirm: true }, Nothing),
  commit: (h: string, p: string, message: string, amend: boolean) => op(h, p, { action: 'commit', message, amend }, GitCommitDataSchema),
  log: (h: string, p: string, skip: number, limit = LOG_PAGE) => op(h, p, { action: 'log', skip, limit }, GitLogDataSchema),
  show: (h: string, p: string, rev: string) => op(h, p, { action: 'show', rev }, GitShowDataSchema),
  branches: (h: string, p: string) => op(h, p, { action: 'branches' }, GitBranchesDataSchema),
  switch: (h: string, p: string, branch: string, create = false) => op(h, p, { action: 'switch', branch, create }, Nothing),
  fetch: (h: string, p: string) => op(h, p, { action: 'fetch' }, GitSyncDataSchema),
  pull: (h: string, p: string, rebase = false) => op(h, p, { action: 'pull', rebase }, GitSyncDataSchema),
  push: (h: string, p: string) => op(h, p, { action: 'push' }, GitSyncDataSchema),
  stashPush: (h: string, p: string) => op(h, p, { action: 'stash_push' }, GitSyncDataSchema),
  stashPop: (h: string, p: string) => op(h, p, { action: 'stash_pop' }, GitSyncDataSchema),
  /** UNI-257: total +/- (staged + unstaged + untracked) for the chat menu's Git row. */
  shortstat: (h: string, p: string) => op(h, p, { action: 'shortstat' }, GitShortstatDataSchema),
};

// ── Chat menu's Git row (UNI-257) ─────────────────────────────────────────

export type GitMenuStat = { add: number; del: number; capped: boolean };
const menuStats = new Map<string, GitMenuStat | null>();

/** The +/- to show beside the chat menu's Git row: null when clean, not a
 *  repo, or the host can't answer (older host → E_SCHEMA; any error). */
export function menuStatOf(res: Result<{ repo: boolean; add: number; del: number; capped: boolean }>): GitMenuStat | null {
  if (!res.ok || !res.value.repo) return null;
  const { add, del, capped } = res.value;
  return add === 0 && del === 0 ? null : { add, del, capped };
}

/** Last known stat for a chat's cwd (shown at once while the menu refreshes it). */
export function cachedMenuStat(hostId: string, path: string): GitMenuStat | null | undefined {
  return menuStats.get(`${hostId}\u0000${path}`);
}

/** Fetches the stat again (on every menu open) and remembers it. */
export async function refreshMenuStat(hostId: string, path: string): Promise<GitMenuStat | null> {
  const stat = menuStatOf(await git.shortstat(hostId, path));
  menuStats.set(`${hostId}\u0000${path}`, stat);
  return stat;
}

/** "1.2k" above 9999 so the row stays short. */
export function compactCount(n: number): string {
  if (n < 10000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 100000 ? 1 : 0).replace(/\.0$/, '')}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
}

export const LOG_PAGE = 50;

// ── Where the Git view opens (Files toolbar chip, chat menu) ─────────────

export type GitTarget = { hostId: string; path: string; /** Where Back returns to. */ from?: 'files' | 'chat' };
const [target, setTarget] = createSignal<GitTarget | undefined>(undefined);
/** The open Git view's target (undefined = closed). */
export const gitTarget = target;
export function openGit(next: GitTarget): void {
  setTarget(next);
}
export function closeGit(): void {
  setTarget(undefined);
}

// ── Pure helpers ──────────────────────────────────────────────────────────

export type GitGroupKey = 'conflict' | 'staged' | 'unstaged' | 'untracked';
export type GitGroup = { key: GitGroupKey; label: string; files: GitFileChange[] };

const GROUP_LABEL: Record<GitGroupKey, string> = {
  conflict: 'Conflicts',
  staged: 'Staged',
  unstaged: 'Changes',
  untracked: 'Untracked',
};

/** Conflicts, Staged, Changes, Untracked — empty groups dropped, files sorted by path. */
export function groupChanges(files: GitFileChange[]): GitGroup[] {
  const order: GitGroupKey[] = ['conflict', 'staged', 'unstaged', 'untracked'];
  return order
    .map((key) => ({
      key,
      label: GROUP_LABEL[key],
      files: files.filter((f) => f.area === key).sort((a, b) => a.path.localeCompare(b.path)),
    }))
    .filter((g) => g.files.length > 0);
}

/** Sum of +/- over a list (binary files count 0). */
export function totals(files: GitFileChange[]): { add: number; del: number } {
  return files.reduce((t, f) => ({ add: t.add + (f.add ?? 0), del: t.del + (f.del ?? 0) }), { add: 0, del: 0 });
}

/** A stable key for a change row (a file can be both staged and unstaged). */
export function changeKey(f: Pick<GitFileChange, 'area' | 'path'>): string {
  return `${f.area}:${f.path}`;
}

/** "a/b/c.ts" → { name: "c.ts", dir: "a/b" }. */
export function splitPath(path: string): { name: string; dir: string } {
  const i = path.lastIndexOf('/');
  return i < 0 ? { name: path, dir: '' } : { name: path.slice(i + 1), dir: path.slice(0, i) };
}

/** Status letter → word for screen readers and titles. */
export function statusWord(status: string): string {
  switch (status) {
    case 'M': return 'modified';
    case 'A': return 'added';
    case 'D': return 'deleted';
    case 'R': return 'renamed';
    case 'C': return 'copied';
    case 'T': return 'type changed';
    case '?': return 'untracked';
    case 'U': return 'conflict';
    default: return 'changed';
  }
}

/** "3m", "2h", "5d", "3w", "4mo", "2y" ago (unix seconds). */
export function relativeTime(unixSeconds: number, now = Date.now()): string {
  const s = Math.max(0, Math.round(now / 1000 - unixSeconds));
  if (s < 60) return 'just now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 14) return `${d}d ago`;
  if (d < 60) return `${Math.floor(d / 7)}w ago`;
  if (d < 365) return `${Math.floor(d / 30)}mo ago`;
  return `${Math.floor(d / 365)}y ago`;
}

/** "↑2 ↓1", "↑2", "", for the branch chip / sync bar. */
export function aheadBehind(repo: Pick<GitRepoData, 'ahead' | 'behind'>): string {
  const parts: string[] = [];
  if (repo.ahead > 0) parts.push(`↑${repo.ahead}`);
  if (repo.behind > 0) parts.push(`↓${repo.behind}`);
  return parts.join(' ');
}

/** The branch chip label: branch name, "detached @abc1234", or "no commits yet". */
export function branchLabel(repo: Pick<GitRepoData, 'branch' | 'head' | 'detached'>): string {
  if (repo.branch) return repo.branch;
  if (repo.head) return `detached @${repo.head}`;
  return 'no branch';
}

/** Commit decorations worth a pill: drop `HEAD -> ` prefixes, keep tags. */
export function refPills(refs: string[]): { label: string; kind: 'head' | 'branch' | 'remote' | 'tag' }[] {
  const out: { label: string; kind: 'head' | 'branch' | 'remote' | 'tag' }[] = [];
  for (const raw of refs) {
    if (raw === 'HEAD') continue;
    if (raw.startsWith('HEAD -> ')) out.push({ label: raw.slice(8), kind: 'head' });
    else if (raw.startsWith('tag: ')) out.push({ label: raw.slice(5), kind: 'tag' });
    else if (raw.includes('/')) out.push({ label: raw, kind: raw.endsWith('/HEAD') ? 'remote' : 'remote' });
    else out.push({ label: raw, kind: 'branch' });
  }
  return out.filter((p) => !p.label.endsWith('/HEAD'));
}

/** Text for the commit button. */
export function commitLabel(staged: number, amend: boolean): string {
  if (amend) return staged > 0 ? `Amend with ${staged} file${staged === 1 ? '' : 's'}` : 'Amend message';
  if (staged === 0) return 'Commit';
  return `Commit ${staged} file${staged === 1 ? '' : 's'}`;
}

/** Whether the commit button can run. */
export function canCommit(message: string, staged: number, amend: boolean, busy: boolean): boolean {
  return !busy && message.trim().length > 0 && (staged > 0 || amend);
}

/** An older host has no `git` op: say so plainly instead of serde's text. */
export function gitErrorText(error: AppError): { title: string; body: string; hint?: string } {
  if (error.code === 'E_SCHEMA' && /unknown variant `git`|unknown op/i.test(error.message)) {
    return { title: 'Update UniPi on the PC', body: 'This PC runs an older unipi-host without git. Re-run the installer on the PC.' };
  }
  if (error.code === 'E_SCOPE') return { title: 'Folder not allowed', body: error.message, hint: error.hint };
  if (error.code === 'E_UNSUPPORTED') return { title: 'git not available', body: error.message, hint: error.hint };
  if (error.code === 'E_TIMEOUT') return { title: 'git took too long', body: error.message, hint: error.hint };
  return { title: 'git said no', body: error.message, hint: error.hint };
}

/** First line of a commit message, for the toast. */
export function subjectOf(message: string): string {
  return message.trim().split('\n')[0]!.trim();
}

/** Ctrl/⌘+Enter (commit shortcut). */
export function isCommitKey(e: { key: string; ctrlKey: boolean; metaKey: boolean; shiftKey?: boolean; altKey?: boolean }): boolean {
  return e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.altKey;
}

/** Status payload with nothing changed. */
export function isClean(status: GitStatusData | undefined): boolean {
  return !!status && status.files.length === 0;
}
