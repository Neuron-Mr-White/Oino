/**
 * The status pane's Git section (WP-A15): the re-homed Git view. Review
 * sources Unstaged / Staged / Branch (vs base) / **Last turn** (Z-96) with
 * unipi grouping, virtualized rows, inline lazy diffs, find-in-changes and
 * the History tab; every unipi write action stays here (Z-97); refresh on
 * `git_changed` hub events only — no polling (Z-98); no graph (Z-99).
 *
 * The component renders inside the `git` side-pane tab (desktop) and the
 * phone's full-screen layer (§3.9) — `StatusPane` provides the chrome.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, For, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  ArrowDownToLine,
  ArrowLeft,
  ArrowUpFromLine,
  Check,
  CloudDownload,
  Copy,
  ExternalLink,
  FolderTree,
  GitBranch,
  GitCommitHorizontal,
  Minus,
  MoreVertical,
  Plus,
  RefreshCw,
  Search,
  Undo2,
  X,
} from 'lucide-solid';
import type { AppError, GitDiffData } from '@unipi/protocol';
import { desktopShell, menuAnchor } from '../../lib/layout';
import { registerCommands } from '../../lib/shell';
import { inTextField } from '../../lib/desktop';
import { setEditorTarget } from '../../lib/editorTarget';
import { parseUnifiedDiff } from '../../lib/markdown';
import { hubs } from '../../lib/session/hub';
import { panes } from '../../lib/panes';
import { currentWorkspace } from '../shell/Workspace';
import { sessions } from '../../lib/session/store';
import type { Turn } from '../../lib/session/turns';
import {
  GIT_SOURCES,
  aheadBehind,
  branchLabel,
  canCommit,
  changeKey,
  commitLabel,
  flattenGroups,
  git,
  gitErrorText,
  isCommitKey,
  lastTurnFiles,
  LOG_PAGE,
  relativeTime,
  sourceGroups,
  splitPath,
  statusWord,
  subjectOf,
  totals,
  type GitBranchFilesData,
  type GitFileChange,
  type GitSource,
  type GitStatusData,
  type GitTurnFilesData,
} from '../../lib/git';
import { EmptyState, isUnreachable, OfflineState } from '../EmptyState';
import { ErrorBanner } from '../ErrorBanner';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { BoardSheet } from '../board/BoardSheet';
import { GitChanges } from '../git/GitChanges';
import { GitHistory } from '../git/GitHistory';
import { GitDiffDetail, GitCommitDetail } from '../git/GitDetail';
import { BranchSheet } from '../git/BranchSheet';

type Selection = { kind: 'file'; change: GitFileChange; preloaded?: GitDiffData } | { kind: 'commit'; hash: string };

function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed || '/';
}

export function GitSection(props: {
  hostId: string;
  /** Where git runs: the session cwd, the Files folder, or `~`. */
  path: string;
  /** The shown session (the "Last turn" source reads its turns index). */
  sessionId?: string;
  cwd?: string;
  /** The phone layer's back button (absent in the pane tab). */
  onBack?: () => void;
  onToast: (text: string) => void;
  /** Initial review source / tab (capture scenarios seed the pane's params). */
  initialSource?: GitSource;
  initialTab?: 'changes' | 'history';
}) {
  const navigate = useNavigate();
  const hostId = props.hostId;
  const [status, setStatus] = createSignal<GitStatusData | undefined>(undefined);
  const [loadError, setLoadError] = createSignal<AppError | undefined>(undefined);
  const [loading, setLoading] = createSignal(true);
  const [source, setSource] = createSignal<GitSource>(props.initialSource ?? 'unstaged');
  const [tab, setTab] = createSignal<'changes' | 'history'>(props.initialTab ?? 'changes');
  const [findOpen, setFindOpen] = createSignal(false);
  const [findQuery, setFindQuery] = createSignal('');
  const [selected, setSelected] = createSignal<Selection | undefined>(undefined);
  const [expandedKey, setExpandedKey] = createSignal<string | undefined>(undefined);
  const [message, setMessage] = createSignal('');
  const [amend, setAmend] = createSignal(false);
  const [busy, setBusy] = createSignal<string | undefined>(undefined);
  const [actionError, setActionError] = createSignal<AppError | undefined>(undefined);
  const [branchSheet, setBranchSheet] = createSignal(false);
  const [discard, setDiscard] = createSignal<GitFileChange[] | undefined>(undefined);
  const [headMenu, setHeadMenu] = createSignal<DOMRect | undefined>(undefined);
  const [rowMenu, setRowMenu] = createSignal<{ change: GitFileChange; anchor: DOMRect } | undefined>(undefined);
  const [commits, setCommits] = createSignal<import('../../lib/git').GitCommitInfo[]>([]);
  const [logMore, setLogMore] = createSignal(false);
  const [logLoading, setLogLoading] = createSignal(false);
  const [logError, setLogError] = createSignal<AppError | undefined>(undefined);
  const [turnData, setTurnData] = createSignal<GitTurnFilesData | undefined>(undefined);
  const [branchData, setBranchData] = createSignal<GitBranchFilesData | undefined>(undefined);
  const [diffTick, setDiffTick] = createSignal(0);
  const [turnLoading, setTurnLoading] = createSignal(false);

  /** Where git runs: the repo root once known, else the folder we opened at. */
  const repoPath = () => status()?.root ?? props.path;
  const repoName = () => basename(status()?.root ?? props.path);
  const isRepo = () => status()?.repo !== false;
  const desktop = () => desktopShell();

  // The shown session's turns (the "Last turn" source); pinned while open.
  const session = createMemo(() => (props.sessionId ? sessions().ensure(hostId, props.sessionId) : undefined));
  const turns = createMemo<readonly Turn[]>(() => session()?.turns() ?? []);
  {
    const release = session()?.acquire();
    if (release) onCleanup(release);
  }

  // ── Data ────────────────────────────────────────────────────────────────

  async function refresh() {
    const repo = await git.repo(hostId, props.path);
    if (!repo.ok) {
      setLoading(false);
      setLoadError(repo.error);
      return;
    }
    if (!repo.value.repo) {
      setLoading(false);
      setLoadError(undefined);
      setStatus({ ...repo.value, files: [], more: false });
      return;
    }
    const result = await git.status(hostId, repo.value.root ?? props.path);
    setLoading(false);
    if (!result.ok) {
      setLoadError(result.error);
      return;
    }
    setLoadError(undefined);
    setStatus(result.value);
    setDiffTick((n) => n + 1);
    // A selected file that's gone (committed, discarded) clears the detail.
    const sel = selected();
    if (sel?.kind === 'file' && source() !== 'branch') {
      const still = result.value.files.find((f) => changeKey(f) === changeKey(sel.change));
      if (still) setSelected({ kind: 'file', change: still, preloaded: sel.preloaded });
      else {
        const other = result.value.files.find((f) => f.path === sel.change.path);
        setSelected(other ? { kind: 'file', change: other } : undefined);
      }
    }
    await loadSource(result.value.root ?? props.path, true);
  }

  /** The Branch / Last turn sources' own round trip (`force` on refresh). */
  async function loadSource(root: string, force = false) {
    if (source() === 'branch' && (force || !branchData())) await loadBranch(root);
    if (source() === 'turn' && (force || !turnData())) await loadTurn(root);
  }

  async function loadBranch(root: string) {
    const result = await git.branchFiles(hostId, root);
    if (result.ok) setBranchData(result.value);
  }

  async function loadTurn(root: string) {
    const files = lastTurnFiles(turns(), props.cwd ?? status()?.root);
    if (!files.length) {
      setTurnData({ repo: false, detached: false, remotes: [], ahead: 0, behind: 0, stashes: 0, files: [] });
      return;
    }
    setTurnLoading(true);
    const result = await git.turnFiles(hostId, root, files);
    setTurnLoading(false);
    if (result.ok) setTurnData(result.value);
  }

  async function loadLog(reset: boolean) {
    if (logLoading()) return;
    setLogLoading(true);
    setLogError(undefined);
    const skip = reset ? 0 : commits().length;
    const result = await git.log(hostId, repoPath(), skip, LOG_PAGE);
    setLogLoading(false);
    if (!result.ok) {
      setLogError(result.error);
      return;
    }
    setCommits(reset ? result.value.commits : [...commits(), ...result.value.commits]);
    setLogMore(result.value.more);
  }

  onMount(() => {
    void refresh().then(() => {
      if (tab() === 'history' && status()?.repo) void loadLog(true);
    });
  });
  createEffect(on(tab, (t) => {
    if (t === 'history' && commits().length === 0 && status()?.repo) void loadLog(true);
  }));
  createEffect(on(source, () => void loadSource(repoPath())));

  // Z-98: refresh on `git_changed` only. The watcher is refcounted host-side;
  // the pane keeps one reference while it is open.
  const [watchedRoot, setWatchedRoot] = createSignal<string | undefined>(undefined);
  createEffect(() => {
    const s = status();
    if (!s?.repo) return;
    const root = s.root ?? props.path;
    const prev = watchedRoot();
    if (root === prev) return;
    setWatchedRoot(root);
    void git.watch(hostId, root, true);
    // The repo root moved (a subfolder of another repo): release the old one.
    if (prev) void git.watch(hostId, prev, false);
  });
  onCleanup(() => {
    const root = watchedRoot();
    if (root) void git.watch(hostId, root, false);
  });
  const hub = createMemo(() => hubs().ensure(hostId));
  createEffect(
    on(
      () => (watchedRoot() ? hub().state.git[watchedRoot()!] : undefined),
      (tick) => {
        if (tick === undefined) return; // armed once the repo root is watched
        void refresh();
      },
      { defer: true },
    ),
  );

  // ── Actions (Z-97: the unipi set, all inside the pane) ──────────────────

  /** Runs one mutating git action; refreshes after, shows errors plainly. */
  async function act<T>(name: string, run: () => Promise<{ ok: true; value: T } | { ok: false; error: AppError }>, done?: (value: T) => void) {
    if (busy()) return;
    setBusy(name);
    setActionError(undefined);
    const result = await run();
    setBusy(undefined);
    if (!result.ok) {
      setActionError(result.error);
    } else {
      done?.(result.value);
    }
    await refresh();
    if (tab() === 'history' || commits().length > 0) void loadLog(true);
  }

  const toggleStage = (change: GitFileChange) => {
    const path = change.path;
    if (change.area === 'staged') void act(`stage:${path}`, () => git.unstage(hostId, repoPath(), [path]));
    else void act(`stage:${path}`, () => git.stage(hostId, repoPath(), [path]));
  };
  const stageGroup = (key: string, files: GitFileChange[]) => {
    if (key === 'g:staged') void act('stage:all', () => git.unstage(hostId, repoPath(), 'all'));
    else void act('stage:all', () => git.stage(hostId, repoPath(), files.map((f) => f.path)));
  };

  function commit() {
    if (!canCommit(message(), stagedCount(), amend(), !!busy())) return;
    const text = message();
    void act('commit', () => git.commit(hostId, repoPath(), text, amend()), (data) => {
      setMessage('');
      setAmend(false);
      props.onToast(`Committed ${data.hash} · ${subjectOf(text)}`);
    });
  }

  async function toggleAmend() {
    const next = !amend();
    setAmend(next);
    if (next && !message().trim() && status()?.head) {
      const head = await git.show(hostId, repoPath(), 'HEAD');
      if (head.ok && amend() && !message().trim()) {
        setMessage(head.value.body ? `${head.value.commit.subject}\n\n${head.value.body}` : head.value.commit.subject);
      }
    }
  }

  const sync = (kind: 'fetch' | 'pull' | 'push') =>
    void act(kind, () => (kind === 'fetch' ? git.fetch(hostId, repoPath()) : kind === 'pull' ? git.pull(hostId, repoPath()) : git.push(hostId, repoPath())), (data) => {
      const first = data.output.split('\n').find((l) => l.trim()) ?? 'Done.';
      props.onToast(kind === 'push' ? `Pushed · ${first}` : kind === 'pull' ? `Pulled · ${first}` : 'Fetched');
    });

  /** The change's absolute path (git status rows are repo-relative). */
  function absPath(change: GitFileChange): string | undefined {
    const root = status()?.root;
    return root ? `${root.replace(/\/+$/, '')}/${change.path}` : undefined;
  }

  function openFile(change: GitFileChange) {
    const path = absPath(change);
    if (!path) return;
    if (desktop()) {
      // A20's Files tab hosts the editor next to the tree on the desktop.
      const ws = currentWorkspace();
      if (!ws) return;
      const dir = path.slice(0, path.lastIndexOf('/'));
      panes().open(ws.scope, 'side', ws.view, { kind: 'files', params: { hostId, root: dir, path } });
      return;
    }
    setEditorTarget({ hostId, path });
    navigate('/files/edit');
  }

  /** Z-96 "reveal in tree": the Files tree rooted at the file's folder, no
   *  editor (its own row action); the phone opens the Files layer there. */
  function revealInTree(change: GitFileChange) {
    const path = absPath(change);
    if (!path) return;
    const dir = path.slice(0, path.lastIndexOf('/'));
    if (desktop()) {
      const ws = currentWorkspace();
      if (!ws) return;
      panes().open(ws.scope, 'side', ws.view, { kind: 'files', params: { hostId, root: dir } });
      props.onToast('Revealed in Files');
      return;
    }
    // The phone opens the Files layer at its last folder; a real tree
    // selection needs A20's Files tree API (follow-up).
    navigate('/files');
  }
  function copyPath(change: GitFileChange) {
    const root = status()?.root ?? '';
    void navigator.clipboard?.writeText(`${root.replace(/\/+$/, '')}/${change.path}`).then(
      () => props.onToast('Path copied'),
      () => props.onToast('Could not copy'),
    );
  }

  // ── The list model ──────────────────────────────────────────────────────

  const sourceFiles = createMemo<GitFileChange[]>(() => {
    const s = source();
    // `branch_files` / `turn_files` rows are flat: the change fields plus `diff`.
    if (s === 'branch') return branchData()?.files ?? [];
    if (s === 'turn') return turnData()?.files ?? [];
    return status()?.files ?? [];
  });
  const sourceLabel = () => (source() === 'branch' && branchData()?.base ? `vs ${branchData()!.base}` : source() === 'turn' ? "Last turn's files" : undefined);
  const groups = createMemo(() => sourceGroups(source(), sourceFiles(), sourceLabel()));
  const flat = createMemo(() => flattenGroups(groups(), findQuery()));
  const rows = () => flat().rows;
  const matches = () => flat().matches;
  const stagedCount = () => (status()?.files ?? []).filter((f) => f.area === 'staged').length;
  const canStage = () => source() === 'unstaged' || source() === 'staged' || source() === 'turn';
  const preloaded = (diffKey: string) => {
    const s = source();
    if (s !== 'branch' && s !== 'turn') return undefined;
    const row = rows().find((r) => r.kind === 'file' && r.diffKey === diffKey);
    if (!row || row.kind !== 'file') return undefined;
    const source2 = s === 'branch' ? branchData() : turnData();
    const hit = (source2?.files ?? []).find((f) => f.path === row.change.path && f.orig === row.change.orig);
    return hit?.diff;
  };
  const selectedKey = () => {
    const sel = selected();
    if (sel?.kind !== 'file') return undefined;
    const hit = rows().find((r) => r.kind === 'file' && changeKey(r.change) === changeKey(sel.change));
    return hit && hit.kind === 'file' ? hit.key : undefined;
  };

  const pick = (row: { kind: 'file'; key: string; change: GitFileChange; diffKey: string } | undefined) => {
    if (!row) return setSelected(undefined);
    const pre = preloaded(row.diffKey);
    setSelected({ kind: 'file', change: row.change, ...(pre ? { preloaded: pre } : {}) });
    if (!desktop()) setFindOpen(false);
  };

  function pickByKey(key: string | undefined) {
    if (!key) return setSelected(undefined);
    const hit = rows().find((r) => r.kind === 'file' && r.key === key);
    pick(hit && hit.kind === 'file' ? hit : undefined);
  }

  const selKey = createMemo(() => {
    const sel = selected();
    return sel ? (sel.kind === 'file' ? `f:${changeKey(sel.change)}` : `c:${sel.hash}`) : undefined;
  });
  const selChange = () => {
    const sel = selected();
    return sel?.kind === 'file' ? sel.change : undefined;
  };
  const selCommit = () => {
    const sel = selected();
    return sel?.kind === 'commit' ? sel.hash : undefined;
  };
  const selPreloaded = () => {
    const sel = selected();
    return sel?.kind === 'file' ? sel.preloaded : undefined;
  };

  // ── Keyboard: ↑/↓ select, Space stage, Ctrl/⌘+Enter commit, / find ─────

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector('.sheet[data-open], .recent-popover') || inTextField(e.target)) return;
      if (e.key === '/' && tab() === 'changes' && !findOpen()) {
        e.preventDefault();
        setFindOpen(true);
        return;
      }
      if (isCommitKey(e)) {
        if (tab() === 'changes') {
          e.preventDefault();
          commit();
        }
        return;
      }
      if (!desktop() || e.ctrlKey || e.metaKey || e.altKey) return;
      if (tab() === 'changes') {
        const list = rows().flatMap((r) => (r.kind === 'file' ? [r] : []));
        const sel = selected();
        const index = sel?.kind === 'file' ? list.findIndex((r) => changeKey(r.change) === changeKey(sel.change)) : -1;
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          if (!list.length) return;
          e.preventDefault();
          const next = e.key === 'ArrowDown' ? Math.min(list.length - 1, index + 1) : Math.max(0, index < 0 ? 0 : index - 1);
          const row = list[next]!;
          pick(row);
          if (desktop()) setExpandedKey(row.diffKey);
        } else if (e.key === ' ' && sel?.kind === 'file' && canStage()) {
          e.preventDefault();
          toggleStage(sel.change);
        }
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const list = commits();
        if (!list.length) return;
        e.preventDefault();
        const sel = selected();
        const index = sel?.kind === 'commit' ? list.findIndex((c) => c.hash === sel.hash) : -1;
        const next = e.key === 'ArrowDown' ? Math.min(list.length - 1, index + 1) : Math.max(0, index < 0 ? 0 : index - 1);
        setSelected({ kind: 'commit', hash: list[next]!.hash });
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  onCleanup(
    registerCommands([
      { id: 'git.commit', title: 'Commit staged changes', group: 'Git', keys: 'Ctrl Enter', icon: () => <GitCommitHorizontal size={16} />, when: () => canCommit(message(), stagedCount(), amend(), !!busy()), run: commit },
      { id: 'git.pull', title: 'Pull', group: 'Git', icon: () => <ArrowDownToLine size={16} />, when: () => !!status()?.upstream, run: () => sync('pull') },
      { id: 'git.push', title: 'Push', group: 'Git', icon: () => <ArrowUpFromLine size={16} />, when: () => !!status()?.repo, run: () => sync('push') },
      { id: 'git.fetch', title: 'Fetch', group: 'Git', icon: () => <CloudDownload size={16} />, when: () => !!status()?.remotes.length, run: () => sync('fetch') },
      { id: 'git.branch', title: 'Switch branch', group: 'Git', icon: () => <GitBranch size={16} />, when: () => !!status()?.repo, run: () => setBranchSheet(true) },
      { id: 'git.stageAll', title: 'Stage all changes', group: 'Git', icon: () => <Plus size={16} />, when: () => !!status()?.files.some((f) => f.area !== 'staged'), run: () => void act('stage:all', () => git.stage(hostId, repoPath(), 'all')) },
      { id: 'git.refresh', title: 'Refresh git status', group: 'Git', icon: () => <RefreshCw size={16} />, run: () => void refresh() },
    ]),
  );

  const syncLine = () => {
    const s = status();
    if (!s?.repo) return props.path;
    const parts = [branchLabel(s)];
    if (s.upstream) parts.push(`${s.upstream}${aheadBehind(s) ? ` ${aheadBehind(s)}` : ''}`);
    else if (s.branch && s.remotes.length) parts.push('not pushed yet');
    if (s.operation) parts.push(`${s.operation} in progress`);
    return parts.join(' · ');
  };

  // ── Render ──────────────────────────────────────────────────────────────

  const toolbar = (
    <div class="git-toolbar" data-testid="git-toolbar">
      <label class="git-source-wrap">
        <span class="sr-only">Review source</span>
        <select
          class="git-source"
          data-testid="git-source"
          value={source()}
          onChange={(e) => {
            setSource(e.currentTarget.value as GitSource);
            setSelected(undefined);
            setExpandedKey(undefined);
          }}
        >
          <For each={GIT_SOURCES.filter((s) => s.id !== 'turn' || !!props.sessionId)}>
            {(s) => <option value={s.id} title={s.hint}>{s.label}</option>}
          </For>
        </select>
      </label>
      <Show when={findOpen()}>
        <input
          type="search"
          class="files-search-input git-find"
          data-testid="git-find"
          placeholder="Find in changes"
          aria-label="Find in changes"
          value={findQuery()}
          onInput={(e) => setFindQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              setFindQuery('');
              setFindOpen(false);
            }
            if (e.key === 'Enter') {
              const list = rows().flatMap((r) => (r.kind === 'file' ? [r] : []));
              if (!list.length) return;
              const sel = selected();
              const at = sel?.kind === 'file' ? list.findIndex((r) => changeKey(r.change) === changeKey(sel.change)) : -1;
              pick(list[(at + 1) % list.length]!);
            }
          }}
        />
        <span class="git-find-count" aria-live="polite">{findQuery().trim() ? `${matches()} match${matches() === 1 ? '' : 'es'}` : ''}</span>
      </Show>
      <button
        type="button"
        class="ghost-icon-button"
        aria-label={findOpen() ? 'Close find' : 'Find in changes'}
        aria-pressed={findOpen()}
        data-active={findOpen() ? '' : undefined}
        onClick={() => {
          setFindOpen(!findOpen());
          if (!findOpen()) setFindQuery('');
        }}
      >
        {findOpen() ? <X size={17} /> : <Search size={17} />}
      </button>
      <button type="button" class="ghost-icon-button" aria-label="Refresh" disabled={!!busy()} onClick={() => void refresh()}>
        <Show when={busy() === 'refresh' || loading()} fallback={<RefreshCw size={17} />}>
          <RefreshCw size={17} class="refresh-spinning" />
        </Show>
      </button>
    </div>
  );

  const tabsRow = (
    <div class="git-tabs-row">
      <div class="seg git-tabs" role="tablist" aria-label="Git view">
        <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'changes'} onClick={() => { setTab('changes'); setSelected(undefined); setExpandedKey(undefined); }}>
          Changes <Show when={(status()?.files.length ?? 0) > 0}><span class="git-tab-count">{status()?.files.length}</span></Show>
        </button>
        <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'history'} onClick={() => { setTab('history'); setSelected(undefined); setExpandedKey(undefined); }}>
          History
        </button>
      </div>
    </div>
  );

  const composer = (
    <form
      class="git-composer"
      data-testid="git-composer"
      onSubmit={(e) => {
        e.preventDefault();
        commit();
      }}
    >
      <textarea
        class="git-message"
        rows={1}
        placeholder={stagedCount() > 0 || amend() ? 'Commit message' : 'Stage changes, then write a message'}
        aria-label="Commit message"
        value={message()}
        onInput={(e) => {
          setMessage(e.currentTarget.value);
          e.currentTarget.style.height = 'auto';
          e.currentTarget.style.height = `${Math.min(e.currentTarget.scrollHeight, 160)}px`;
        }}
      />
      <div class="git-composer-row">
        <button type="button" class="files-pill git-amend" classList={{ active: amend() }} aria-pressed={amend()} disabled={!status()?.head} onClick={() => void toggleAmend()}>
          Amend
        </button>
        <Show when={desktop()}>
          <span class="git-kbd-hint">Ctrl+Enter</span>
        </Show>
        <button type="submit" class="btn btn-primary btn-sm git-commit-btn" disabled={!canCommit(message(), stagedCount(), amend(), !!busy())}>
          <Show when={busy() === 'commit'} fallback={<GitCommitHorizontal size={16} />}>
            <RefreshCw size={15} class="refresh-spinning" />
          </Show>
          {commitLabel(stagedCount(), amend())}
        </button>
      </div>
    </form>
  );

  const body = () => {
    const err = loadError();
    if (err && !status()) {
      if (isUnreachable(err.code)) return <OfflineState message={err.message} hint={err.hint} onRetry={() => { setLoading(true); void refresh(); }} />;
      const text = gitErrorText(err);
      return (
        <EmptyState
          tone="error"
          title={text.title}
          body={text.hint ?? text.body}
          testId="git-load-error"
          action={<button type="button" class="btn btn-secondary" onClick={() => { setLoading(true); void refresh(); }}><RefreshCw size={16} /> Retry</button>}
        />
      );
    }
    if (loading() && !status()) return <p class="git-note git-loading">Reading the repository…</p>;
    if (!isRepo())
      return (
        <EmptyState
          icon={<GitBranch size={24} />}
          title="Not a git repository"
          body={`${props.path} isn't inside a git repo.`}
          testId="git-not-repo"
        />
      );
    if (tab() === 'history') {
      return (
        <>
          <Show when={logError()}>
            {(e) => <ErrorBanner error={{ ...e(), retryable: true }} retry={() => void loadLog(commits().length === 0)} />}
          </Show>
          <GitHistory
            commits={commits()}
            selected={selCommit()}
            more={logMore()}
            loading={logLoading()}
            onSelect={(hash) => setSelected({ kind: 'commit', hash })}
            onMore={() => void loadLog(false)}
          />
          <Show when={desktop() && selCommit()}>
            {(hash) => (
              <div class="git-detail-overlay" data-testid="git-commit-overlay">
                <GitCommitDetail hostId={hostId} repo={repoPath()} hash={hash()} onClose={() => setSelected(undefined)} />
              </div>
            )}
          </Show>
        </>
      );
    }
    const cleanEmpty = sourceFiles().length === 0 && !findQuery().trim();
    const sourceEmpty = cleanEmpty
      ? source() === 'turn'
        ? turnLoading()
          ? { title: 'Reading the last turn…', body: undefined as string | undefined }
          : { title: 'No files from the last turn', body: 'Write or edit a file in a turn and it lands here.' }
        : source() === 'branch'
          ? { title: branchData()?.base ? `Even with ${branchData()!.base}` : 'No base to compare', body: branchData()?.base ? 'Nothing changed on this branch.' : 'Give the branch an upstream (or a main) to compare against.' }
          : { title: 'Nothing to commit', body: status()?.head ? 'The working tree is clean.' : 'No commits yet — add some files.' }
      : undefined;
    return (
      <>
        <Show when={actionError()}>
          {(e) => {
            const text = gitErrorText(e());
            return (
              <div class="git-error" data-testid="git-action-error">
                <ErrorBanner error={{ code: e().code, message: text.body, hint: text.hint, retryable: false }} dismiss={() => setActionError(undefined)} />
              </div>
            );
          }}
        </Show>
        <GitChanges
          rows={rows()}
          expandedKey={expandedKey()}
          selectedKey={selectedKey()}
          hostId={hostId}
          repo={repoPath()}
          tick={diffTick()}
          preloaded={preloaded}
          sourceLabel={sourceLabel()}
          busy={busy()}
          canStage={canStage()}
          onExpand={setExpandedKey}
          onSelect={pickByKey}
          onToggleStage={toggleStage}
          onStageGroup={canStage() ? stageGroup : undefined}
          onRowMenu={(change, anchor) => setRowMenu({ change, anchor })}
        >
          <Show when={sourceEmpty} keyed>
            {(empty) => (
              <EmptyState
                compact
                icon={<Check size={22} />}
                title={empty.title}
                body={empty.body}
                testId={source() === 'turn' ? 'git-turn-empty' : source() === 'branch' ? 'git-branch-empty' : 'git-clean'}
              />
            )}
          </Show>
          <Show when={findQuery().trim() && matches() === 0}>
            <EmptyState compact icon={<Search size={22} />} title="No matches" body={`Nothing in this source matches “${findQuery().trim()}”.`} testId="git-find-empty" />
          </Show>
          <Show when={status()?.more}>
            <p class="git-note">Showing the first 2000 changed files.</p>
          </Show>
        </GitChanges>
        <Show when={canStage()}>{composer}</Show>
      </>
    );
  };

  return (
    <div class="git-section" data-testid="git-section">
      <div class="git-section-head">
        <Show when={props.onBack}>
          <button type="button" class="ghost-icon-button" aria-label="Close Git" onClick={props.onBack}>
            <ArrowLeft size={22} />
          </button>
        </Show>
        <div class="git-head-title">
          <span class="git-head-name truncate">{repoName()}</span>
          <span class="git-head-sub truncate" title={syncLine()}>{syncLine()}</span>
        </div>
        <span class="git-section-head-actions">
          <button
            type="button"
            class="ghost-icon-button git-sync-btn"
            aria-label={status()?.behind ? `Pull ${status()?.behind} commits` : 'Pull'}
            title="Pull (fast-forward only)"
            disabled={!!busy()}
            onClick={() => sync('pull')}
          >
            <Show when={busy() === 'pull'} fallback={<ArrowDownToLine size={19} />}>
              <RefreshCw size={18} class="refresh-spinning" />
            </Show>
            <Show when={(status()?.behind ?? 0) > 0}>
              <span class="git-sync-count">{status()?.behind}</span>
            </Show>
          </button>
          <button
            type="button"
            class="ghost-icon-button git-sync-btn"
            aria-label={status()?.ahead ? `Push ${status()?.ahead} commits` : 'Push'}
            title="Push"
            disabled={!!busy()}
            onClick={() => sync('push')}
          >
            <Show when={busy() === 'push'} fallback={<ArrowUpFromLine size={19} />}>
              <RefreshCw size={18} class="refresh-spinning" />
            </Show>
            <Show when={(status()?.ahead ?? 0) > 0}>
              <span class="git-sync-count">{status()?.ahead}</span>
            </Show>
          </button>
          <button
            type="button"
            class="ghost-icon-button"
            aria-label="Git menu"
            aria-expanded={!!headMenu()}
            onClick={(e) => setHeadMenu(e.currentTarget.getBoundingClientRect())}
          >
            <MoreVertical size={19} />
          </button>
        </span>
      </div>
      {toolbar}
      {tabsRow}
      {body()}
      <Show when={!desktop() && selected()}>
        <div class="git-detail-layer" data-testid="git-detail-layer">
          <Show when={selKey()} keyed>
            {(key) =>
              key.startsWith('f:') ? (
                <GitDiffDetail
                  hostId={hostId}
                  repo={repoPath()}
                  change={selChange()!}
                  tick={diffTick()}
                  preloaded={selPreloaded()}
                  busy={!!busy()}
                  onClose={() => setSelected(undefined)}
                  onToggle={canStage() ? () => toggleStage(selChange()!) : undefined}
                  onDiscard={canStage() && selChange()?.area !== 'staged' ? () => setDiscard([selChange()!]) : undefined}
                  onOpen={() => openFile(selChange()!)}
                />
              ) : (
                <GitCommitDetail hostId={hostId} repo={repoPath()} hash={key.slice(2)} onClose={() => setSelected(undefined)} />
              )
            }
          </Show>
        </div>
      </Show>

      <Show when={headMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} alignEnd label="Git menu" width={250} onDismiss={() => setHeadMenu(undefined)}>
            <MenuRow icon={<GitBranch size={18} />} label="Switch branch…" onSelect={() => { setHeadMenu(undefined); setBranchSheet(true); }} />
            <MenuRow icon={<CloudDownload size={18} />} label="Fetch" disabled={!status()?.remotes.length} onSelect={() => { setHeadMenu(undefined); sync('fetch'); }} />
            <MenuRow icon={<ArrowDownToLine size={18} />} label="Pull with rebase" disabled={!status()?.upstream} onSelect={() => { setHeadMenu(undefined); void act('pull', () => git.pull(hostId, repoPath(), true), () => props.onToast('Pulled (rebase)')); }} />
            <MenuSeparator />
            <MenuRow icon={<Undo2 size={18} />} label="Stash changes" disabled={!status()?.files.length} onSelect={() => { setHeadMenu(undefined); void act('stash', () => git.stashPush(hostId, repoPath()), () => props.onToast('Changes stashed')); }} />
            <MenuRow
              icon={<Undo2 size={18} />}
              label="Pop stash"
              disabled={!status()?.stashes}
              onSelect={() => { setHeadMenu(undefined); void act('stash', () => git.stashPop(hostId, repoPath()), () => props.onToast('Stash applied')); }}
            />
            <MenuSeparator />
            <MenuRow icon={<RefreshCw size={18} />} label="Refresh" onSelect={() => { setHeadMenu(undefined); void refresh(); }} />
          </AnchoredMenu>
        )}
      </Show>

      <Show when={rowMenu()}>
        {(m) => (
          <AnchoredMenu anchor={m().anchor} label="File actions" onDismiss={() => setRowMenu(undefined)}>
            <MenuRow
              icon={m().change.area === 'staged' ? <Minus size={18} /> : <Plus size={18} />}
              label={m().change.area === 'staged' ? 'Unstage' : m().change.area === 'conflict' ? 'Mark resolved' : 'Stage'}
              disabled={!canStage()}
              onSelect={() => { const c = m().change; setRowMenu(undefined); toggleStage(c); }}
            />
            <MenuRow icon={<FolderTree size={18} />} label="Reveal in tree" onSelect={() => { const c = m().change; setRowMenu(undefined); revealInTree(c); }} />
            <MenuRow icon={<ExternalLink size={18} />} label="Open in editor" disabled={m().change.status === 'D'} onSelect={() => { const c = m().change; setRowMenu(undefined); openFile(c); }} />
            <MenuRow icon={<Copy size={18} />} label="Copy path" onSelect={() => { const c = m().change; setRowMenu(undefined); copyPath(c); }} />
            <Show when={canStage() && m().change.area !== 'staged'}>
              <MenuSeparator />
              <MenuRow icon={<Undo2 size={18} />} tone="danger" label={m().change.area === 'untracked' ? 'Delete file…' : 'Discard changes…'} onSelect={() => { const c = m().change; setRowMenu(undefined); setDiscard([c]); }} />
            </Show>
          </AnchoredMenu>
        )}
      </Show>

      <BranchSheet
        open={branchSheet()}
        hostId={hostId}
        repo={repoPath()}
        current={status()?.branch}
        onClose={() => setBranchSheet(false)}
        onSwitched={(name, created) => {
          setBranchSheet(false);
          props.onToast(created ? `Created and switched to ${name}` : `Switched to ${name}`);
          setCommits([]);
          void refresh().then(() => { if (tab() === 'history') void loadLog(true); });
        }}
      />

      <BoardSheet open={!!discard()} onClose={() => setDiscard(undefined)} label={discard()?.[0]?.area === 'untracked' ? 'Delete file?' : 'Discard changes?'} testId="git-discard-sheet">
        <p class="git-sheet-text">
          <Show when={discard()?.[0]?.area === 'untracked'} fallback={<>Throw away your changes to <strong class="git-mono">{discard()?.[0]?.path}</strong>? </>}>
            Delete the untracked file <strong class="git-mono">{discard()?.[0]?.path}</strong>?{' '}
          </Show>
          This can't be undone.
        </p>
        <div class="dialog-actions">
          <button type="button" class="btn btn-secondary" onClick={() => setDiscard(undefined)}>Cancel</button>
          <button
            type="button"
            class="btn btn-danger"
            onClick={() => {
              const list = discard();
              setDiscard(undefined);
              if (!list) return;
              void act('discard', () => git.discard(hostId, repoPath(), list.map((c) => c.path)), () => props.onToast(list[0]!.area === 'untracked' ? 'File deleted' : 'Changes discarded'));
            }}
          >
            {discard()?.[0]?.area === 'untracked' ? 'Delete' : 'Discard'}
          </button>
        </div>
      </BoardSheet>
    </div>
  );
}

export { basename };
