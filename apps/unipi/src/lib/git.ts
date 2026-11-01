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
  GitBranchFilesDataSchema,
  GitCommitDataSchema,
  GitDiffDataSchema,
  GitLogDataSchema,
  GitRepoDataSchema,
  GitShowDataSchema,
  GitStatusDataSchema,
  GitSyncDataSchema,
  GitShortstatDataSchema,
  GitTurnFilesDataSchema,
  GitWatchDataSchema,
  GIT_PATHS_MAX,
  type AppError,
  type GitBranchFilesData,
  type GitCmd,
  type GitFileChange,
  type GitRepoData,
  type GitStatusData,
  type GitTurnFilesData,
} from '@unipi/protocol';
import type { Result } from './ipc';
import { hostCall } from './host/factory';
import { desktopShell } from './layout';
import { toolCategory } from './chat/model';
import type { Turn } from './session/turns';

export type { GitFileChange, GitRepoData, GitStatusData, GitCommitInfo, GitBranch, GitShowData, GitDiffData, GitTurnFile, GitTurnFilesData, GitBranchFilesData } from '@unipi/protocol';

function op<T>(hostId: string, path: string, cmd: GitCmd, schema: z.ZodType<T>, outside?: boolean): Promise<Result<T>> {
  return hostCall(hostId, 'git_op', { path, outside, cmd }, schema);
}

const Nothing = z.unknown();

export const git = {
  repo: (h: string, p: string) => op(h, p, { action: 'repo' }, GitRepoDataSchema),
  status: (h: string, p: string) => op(h, p, { action: 'status' }, GitStatusDataSchema),
  diff: (h: string, p: string, file: string, staged: boolean, untracked: boolean) =>
    op(h, p, { action: 'diff', file, staged, untracked }, GitDiffDataSchema),
  /** Z-96 "Last turn": rows + diffs of the files the last turn touched. */
  turnFiles: (h: string, p: string, files: string[], opts?: { staged?: boolean; untracked?: boolean }) =>
    op(h, p, { action: 'turn_files', files, ...opts }, GitTurnFilesDataSchema),
  /** Z-96 "Branch (vs base)": rows + diffs vs the base ref (upstream, else main/master). */
  branchFiles: (h: string, p: string, base?: string) =>
    op(h, p, base ? { action: 'branch_files', base } : { action: 'branch_files' }, GitBranchFilesDataSchema),
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
  /** Z-98: refcount the host's repo watcher; `git_changed` hub events follow. */
  watch: (h: string, root: string, on = true) => hostCall(h, 'git_watch', { root, on }, GitWatchDataSchema),
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

// ── Where the Git view opens (Z-100: one Git UI) ───────────────────────

export type GitTarget = {
  hostId: string;
  path: string;
  /** Where Back returns to (phone layer). */
  from?: 'files' | 'chat';
  /** The shown session: the "Last turn" source reads its turns index (phone layer). */
  sessionId?: string;
  /** Open on a review source / tab (the turn footer's "Last turn" entry, captures). */
  source?: GitSource;
  tab?: 'changes' | 'history';
};
const [target, setTarget] = createSignal<GitTarget | undefined>(undefined);
/** The phone layer's target (undefined = closed); the desktop pane owns its own tab. */
export const gitTarget = target;

/** Opens the `git` tab in the workspace's side pane; installed by the shell
 *  (AppShell) so `lib/git` stays free of component imports. Returns false
 *  when no conversation workspace is shown (fall back to the layer). */
let desktopOpener: ((target: GitTarget) => boolean) | undefined;
export function setGitDesktopOpener(open: ((target: GitTarget) => boolean) | undefined): void {
  desktopOpener = open;
}

/** Opens the Git view: the `git` side-pane tab on the desktop, this PC's
 *  full-screen layer on the phone (§3.9). One Git UI, however it is opened
 *  (Files branch chip, chat ⋯ Git, capsule, `+` launcher, Cmd/Ctrl+Alt+B). */
export function openGit(next: GitTarget): void {
  if (desktopShell() && desktopOpener?.(next)) return;
  setTarget(next);
}

/** `gitTarget` as the pane/section params (source/tab ride to the section). */
export function gitParams(target: GitTarget): Record<string, unknown> {
  return {
    path: target.path,
    ...(target.source ? { source: target.source } : {}),
    ...(target.tab ? { tab: target.tab } : {}),
  };
}
export function closeGit(): void {
  setTarget(undefined);
}

/** Z-96 "Last turn": the repo-relative paths the last turn's write/edit
 *  tools touched, deduped, capped at the protocol's path limit. Absolute
 *  paths outside `cwd` (another repo, the editor's temp file) are skipped. */
export function lastTurnFiles(turns: readonly Turn[], cwd?: string): string[] {
  const turn = [...turns].reverse().find((t) => t.kind === 'turn');
  if (!turn) return [];
  const base = cwd?.replace(/\/+$/, '');
  const out: string[] = [];
  const seen = new Set<string>();
  for (const segment of turn.segments) {
    for (const item of segment.work) {
      if (out.length >= GIT_PATHS_MAX) return out;
      if (item.kind !== 'tool' || toolCategory(item.tool.name) !== 'edit') continue;
      const args = item.tool.args;
      const raw = typeof args.path === 'string' && args.path.trim()
        ? args.path
        : typeof args.file_path === 'string' && args.file_path.trim()
          ? args.file_path
          : '';
      if (!raw) continue;
      let rel = raw.trim();
      if (base && rel.startsWith(`${base}/`)) rel = rel.slice(base.length + 1);
      else if (rel.startsWith('/') || rel.startsWith('~')) continue;
      if (!rel || rel.startsWith('../') || rel === '..') continue;
      if (!seen.has(rel)) {
        seen.add(rel);
        out.push(rel);
      }
    }
  }
  return out;
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

// ── Review sources (Z-96: ZCode sources + unipi grouping) ─────────────────

export type GitSource = 'unstaged' | 'staged' | 'branch' | 'turn';
export type SourceGroup = { key: string; label: string; files: GitFileChange[] };

export const GIT_SOURCES: { id: GitSource; label: string; hint: string }[] = [
  { id: 'unstaged', label: 'Unstaged', hint: 'Changes in the working tree' },
  { id: 'staged', label: 'Staged', hint: 'Changes in the index' },
  { id: 'branch', label: 'Branch', hint: 'This branch vs its base' },
  { id: 'turn', label: 'Last turn', hint: 'Files the last turn touched' },
];

/** The unipi grouping (Conflicts/Staged/Changes/Untracked) inside a source:
 *  Unstaged = worktree + untracked + conflicts; Branch/Last turn = one group
 *  of their rows (no stage semantics, sorted by path). */
export function sourceGroups(source: GitSource, files: GitFileChange[], label?: string): SourceGroup[] {
  if (source === 'unstaged') return groupChanges(files.filter((f) => f.area !== 'staged'));
  if (source === 'staged') return groupChanges(files.filter((f) => f.area === 'staged'));
  const rows = [...files].sort((a, b) => a.path.localeCompare(b.path));
  return rows.length ? [{ key: source, label: label ?? (source === 'branch' ? 'Branch' : 'Last turn'), files: rows }] : [];
}

/** One flattened row of the virtualized changes list. */
export type GitRow =
  | { kind: 'group'; key: `g:${string}`; label: string; files: GitFileChange[] }
  | { kind: 'file'; key: `f:${string}`; change: GitFileChange; diffKey: string }; // diffKey: inline-diff row id

/** Groups + files flattened for the virtualizer, a find filter applied. */
export function flattenGroups(groups: SourceGroup[], query = ''): { rows: GitRow[]; matches: number } {
  const q = query.trim().toLowerCase();
  const rows: GitRow[] = [];
  let matches = 0;
  for (const g of groups) {
    const kept = q ? g.files.filter((f) => f.path.toLowerCase().includes(q)) : g.files;
    matches += kept.length;
    if (!kept.length) continue;
    rows.push({ kind: 'group', key: `g:${g.key}`, label: g.label, files: kept });
    for (const f of kept) rows.push({ kind: 'file', key: `f:${g.key}:${changeKey(f)}`, change: f, diffKey: `d:${g.key}:${changeKey(f)}` });
  }
  return { rows, matches };
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
    return { title: 'Update UniPi on the PC', body: 'This PC runs an older agent host without git. Update UniPi on the PC.' };
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
