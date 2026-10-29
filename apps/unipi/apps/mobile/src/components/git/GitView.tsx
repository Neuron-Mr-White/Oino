/**
 * Git view (UNI-217): basic git for the repo containing a folder on the PC.
 * Opened from the Files toolbar's branch chip or a chat's ⋯ → Git (session
 * cwd); mounted once in main.tsx, driven by `lib/git.ts` `gitTarget()`.
 *
 * Phone: a full-screen layer — 56px detail header (back, repo + branch,
 * pull / push / ⋯), Changes | History tabs, the changes list grouped
 * Conflicts / Staged / Changes / Untracked with +/- counts and stage
 * toggles, and the commit composer at the bottom. A file or commit opens
 * full screen over it. Desktop: the same layer right of the sidebar with
 * list | detail (diff / commit) side by side; ↑/↓ select, Space stages,
 * Ctrl/⌘+Enter commits, Esc closes.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import {
  ArrowDownToLine,
  ArrowLeft,
  ArrowUpFromLine,
  Archive,
  ArchiveRestore,
  Check,
  CloudDownload,
  Copy,
  ExternalLink,
  GitBranch,
  GitCommitHorizontal,
  Minus,
  MoreVertical,
  Plus,
  RefreshCw,
  Undo2,
  X,
} from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { pushBack } from '../../lib/back';
import { desktopShell, menuAnchor } from '../../lib/layout';
import { registerCommands } from '../../lib/shell';
import { inTextField } from '../../lib/desktop';
import { setEditorTarget } from '../../lib/editorTarget';
import { parseUnifiedDiff, renderDiffHtml } from '../../lib/markdown';
import {
  aheadBehind,
  branchLabel,
  canCommit,
  changeKey,
  closeGit,
  commitLabel,
  git,
  gitErrorText,
  gitTarget,
  groupChanges,
  isCommitKey,
  LOG_PAGE,
  refPills,
  relativeTime,
  splitPath,
  statusWord,
  subjectOf,
  totals,
  type GitBranch as Branch,
  type GitCommitInfo,
  type GitFileChange,
  type GitShowData,
  type GitStatusData,
  type GitTarget,
} from '../../lib/git';
import { EmptyState, isUnreachable, OfflineState } from '../EmptyState';
import { ErrorBanner } from '../ErrorBanner';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { BoardSheet } from '../board/BoardSheet';
import './git.css';

export function GitView() {
  return (
    <Show when={gitTarget()} keyed>
      {(target) => <GitLayer target={target} />}
    </Show>
  );
}

type Selection = { kind: 'file'; change: GitFileChange } | { kind: 'commit'; hash: string };

function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  return trimmed.slice(trimmed.lastIndexOf('/') + 1) || trimmed || '/';
}

/** "+12 −3" in the chat's changes-drawer colours. */
export function Stat(props: { add?: number; del?: number; binary?: boolean }) {
  return (
    <span class="git-stat">
      <Show when={!props.binary} fallback={<span class="git-stat-bin">bin</span>}>
        <Show when={(props.add ?? 0) > 0 || (props.del ?? 0) === 0}>
          <span class="add">+{props.add ?? 0}</span>
        </Show>
        <Show when={(props.del ?? 0) > 0}>
          <span class="del">−{props.del}</span>
        </Show>
      </Show>
    </span>
  );
}

function GitLayer(props: { target: GitTarget }) {
  const navigate = useNavigate();
  const hostId = props.target.hostId;
  const [status, setStatus] = createSignal<GitStatusData | undefined>(undefined);
  const [loadError, setLoadError] = createSignal<AppError | undefined>(undefined);
  const [loading, setLoading] = createSignal(true);
  const [tab, setTab] = createSignal<'changes' | 'history'>('changes');
  const [selected, setSelected] = createSignal<Selection | undefined>(undefined);
  const [message, setMessage] = createSignal('');
  const [amend, setAmend] = createSignal(false);
  const [busy, setBusy] = createSignal<string | undefined>(undefined);
  const [actionError, setActionError] = createSignal<AppError | undefined>(undefined);
  const [toast, setToast] = createSignal<string | undefined>(undefined);
  const [branchSheet, setBranchSheet] = createSignal(false);
  const [discard, setDiscard] = createSignal<GitFileChange[] | undefined>(undefined);
  const [headMenu, setHeadMenu] = createSignal<DOMRect | undefined>(undefined);
  const [rowMenu, setRowMenu] = createSignal<{ change: GitFileChange; anchor: DOMRect } | undefined>(undefined);
  const [commits, setCommits] = createSignal<GitCommitInfo[]>([]);
  const [logMore, setLogMore] = createSignal(false);
  const [logLoading, setLogLoading] = createSignal(false);
  const [logError, setLogError] = createSignal<AppError | undefined>(undefined);
  const [diffTick, setDiffTick] = createSignal(0);

  /** Where git runs: the repo root once known, else the folder we were opened at. */
  const repoPath = () => status()?.root ?? props.target.path;
  const groups = createMemo(() => groupChanges(status()?.files ?? []));
  const flat = createMemo(() => groups().flatMap((g) => g.files));
  const stagedCount = () => (status()?.files ?? []).filter((f) => f.area === 'staged').length;
  const isRepo = () => status()?.repo !== false;

  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  const showToast = (text: string) => {
    setToast(text);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => setToast(undefined), 3000);
  };
  onCleanup(() => clearTimeout(toastTimer));

  async function refresh() {
    const repo = await git.repo(hostId, props.target.path);
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
    const result = await git.status(hostId, repo.value.root ?? props.target.path);
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
    if (sel?.kind === 'file') {
      const still = result.value.files.find((f) => changeKey(f) === changeKey(sel.change));
      if (still) setSelected({ kind: 'file', change: still });
      else {
        const other = result.value.files.find((f) => f.path === sel.change.path);
        setSelected(other ? { kind: 'file', change: other } : undefined);
      }
    }
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
    void refresh();
    onCleanup(pushBack(close));
    const onVisible = () => {
      if (document.visibilityState === 'visible' && !busy()) void refresh();
    };
    document.addEventListener('visibilitychange', onVisible);
    onCleanup(() => document.removeEventListener('visibilitychange', onVisible));
  });
  createEffect(on(tab, (t) => {
    if (t === 'history' && commits().length === 0 && status()?.repo) void loadLog(true);
  }));

  function close() {
    closeGit();
  }

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
  const stageGroup = (key: string) => {
    if (key === 'staged') void act('stage:all', () => git.unstage(hostId, repoPath(), 'all'));
    else {
      const paths = (status()?.files ?? []).filter((f) => f.area === key).map((f) => f.path);
      void act('stage:all', () => git.stage(hostId, repoPath(), paths));
    }
  };

  function commit() {
    if (!canCommit(message(), stagedCount(), amend(), !!busy())) return;
    const text = message();
    void act('commit', () => git.commit(hostId, repoPath(), text, amend()), (data) => {
      setMessage('');
      setAmend(false);
      showToast(`Committed ${data.hash} · ${subjectOf(text)}`);
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
      showToast(kind === 'push' ? `Pushed · ${first}` : kind === 'pull' ? `Pulled · ${first}` : 'Fetched');
    });

  function openFile(change: GitFileChange) {
    const root = status()?.root;
    if (!root) return;
    setEditorTarget({ hostId, path: `${root.replace(/\/+$/, '')}/${change.path}` });
    close();
    navigate('/files/edit');
  }
  function copyPath(change: GitFileChange) {
    const root = status()?.root ?? '';
    void navigator.clipboard?.writeText(`${root.replace(/\/+$/, '')}/${change.path}`).then(
      () => showToast('Path copied'),
      () => showToast('Could not copy'),
    );
  }

  // ── Desktop keyboard: ↑/↓ select, Space stage, Ctrl/⌘+Enter commit ──
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector('.sheet, .recent-popover')) return;
      if (isCommitKey(e)) {
        if (tab() === 'changes') {
          e.preventDefault();
          commit();
        }
        return;
      }
      if (!desktopShell() || inTextField(e.target) || e.ctrlKey || e.metaKey || e.altKey) return;
      if (tab() === 'changes') {
        const list = flat();
        const sel = selected();
        const index = sel?.kind === 'file' ? list.findIndex((f) => changeKey(f) === changeKey(sel.change)) : -1;
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          if (!list.length) return;
          e.preventDefault();
          const next = e.key === 'ArrowDown' ? Math.min(list.length - 1, index + 1) : Math.max(0, index < 0 ? 0 : index - 1);
          setSelected({ kind: 'file', change: list[next]! });
          document.querySelector<HTMLElement>(`[data-git-key="${CSS.escape(changeKey(list[next]!))}"]`)?.scrollIntoView?.({ block: "nearest" });
        } else if (e.key === ' ' && sel?.kind === 'file') {
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
        document.querySelector<HTMLElement>(`[data-git-commit="${list[next]!.hash}"]`)?.scrollIntoView?.({ block: "nearest" });
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
      { id: 'git.close', title: 'Close Git', group: 'Git', keys: 'Esc', icon: () => <X size={16} />, run: close },
    ]),
  );

  const repoName = () => basename(status()?.root ?? props.target.path);
  const syncLine = () => {
    const s = status();
    if (!s?.repo) return props.target.path;
    const parts = [branchLabel(s)];
    if (s.upstream) parts.push(`${s.upstream}${aheadBehind(s) ? ` ${aheadBehind(s)}` : ''}`);
    else if (s.branch && s.remotes.length) parts.push('not pushed yet');
    if (s.operation) parts.push(`${s.operation} in progress`);
    return parts.join(' · ');
  };

  const detailOpen = () => !!selected();

  const head = (
    <header class="git-head detail-head">
      <button type="button" class="ghost-icon-button" aria-label="Close Git" onClick={close}>
        <ArrowLeft size={22} />
      </button>
      <div class="git-head-title">
        <span class="git-head-name truncate">{repoName()}</span>
        <span class="git-head-sub truncate" title={syncLine()}>{syncLine()}</span>
      </div>
      <Show when={status()?.repo}>
        <button
          type="button"
          class="ghost-icon-button git-sync-btn"
          aria-label={status()?.behind ? `Pull ${status()?.behind} commits` : 'Pull'}
          title="Pull (fast-forward only)"
          disabled={!!busy()}
          onClick={() => sync('pull')}
        >
          <Show when={busy() === 'pull'} fallback={<ArrowDownToLine size={20} />}>
            <RefreshCw size={19} class="refresh-spinning" />
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
          <Show when={busy() === 'push'} fallback={<ArrowUpFromLine size={20} />}>
            <RefreshCw size={19} class="refresh-spinning" />
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
          <MoreVertical size={20} />
        </button>
      </Show>
    </header>
  );

  const changesList = () => (
    <div class="git-list-scroll" data-testid="git-changes">
      <Show when={status()?.files.length === 0}>
        <EmptyState compact icon={<Check size={22} />} title="Nothing to commit" body={status()?.head ? 'The working tree is clean.' : 'No commits yet — add some files.'} testId="git-clean" />
      </Show>
      <For each={groups()}>
        {(group) => (
          <section class="git-group" aria-label={group.label}>
            <div class="section-row git-group-head">
              <span class="section-label">
                {group.label} <span class="git-group-count">{group.files.length}</span>
              </span>
              <span class="git-group-tools">
                <Stat {...totals(group.files)} />
                <Show when={group.key !== 'conflict'}>
                  <button type="button" class="git-link" disabled={!!busy()} onClick={() => stageGroup(group.key)}>
                    {group.key === 'staged' ? 'Unstage all' : 'Stage all'}
                  </button>
                </Show>
              </span>
            </div>
            <For each={group.files}>
              {(change) => {
                const parts = () => splitPath(change.path);
                const key = changeKey(change);
                const isSelected = () => {
                  const sel = selected();
                  return sel?.kind === 'file' && changeKey(sel.change) === key;
                };
                return (
                  <div
                    role="button"
                    tabIndex={0}
                    class="git-row"
                    data-testid="git-row"
                    data-git-key={key}
                    data-selected={isSelected() ? '' : undefined}
                    aria-label={`${change.path}, ${statusWord(change.status)}`}
                    onClick={() => setSelected({ kind: 'file', change })}
                    onKeyDown={(e) => {
                      if (desktopShell()) return;
                      if (e.key === 'Enter') setSelected({ kind: 'file', change });
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setRowMenu({ change, anchor: menuAnchor(e, e.currentTarget as HTMLElement) });
                    }}
                  >
                    <span class="files-git-badge git-badge" data-git={change.status} title={statusWord(change.status)}>{change.status}</span>
                    <span class="git-row-main">
                      <span class="git-row-name truncate">{parts().name}</span>
                      <Show when={parts().dir || change.orig}>
                        <span class="git-row-dir truncate">{change.orig ? `from ${change.orig}` : parts().dir}</span>
                      </Show>
                    </span>
                    <Stat add={change.add} del={change.del} binary={change.binary} />
                    <button
                      type="button"
                      class="ghost-icon-button git-stage-btn"
                      data-staged={change.area === 'staged' ? '' : undefined}
                      aria-label={change.area === 'staged' ? `Unstage ${change.path}` : change.area === 'conflict' ? `Mark ${change.path} resolved` : `Stage ${change.path}`}
                      disabled={!!busy()}
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleStage(change);
                      }}
                    >
                      <Show when={busy() === `stage:${change.path}`} fallback={change.area === 'staged' ? <Minus size={18} /> : <Plus size={18} />}>
                        <RefreshCw size={16} class="refresh-spinning" />
                      </Show>
                    </button>
                  </div>
                );
              }}
            </For>
          </section>
        )}
      </For>
      <Show when={status()?.more}>
        <p class="git-note">Showing the first 2000 changed files.</p>
      </Show>
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
        <Show when={desktopShell()}>
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

  const historyList = () => (
    <div class="git-list-scroll" data-testid="git-history">
      <Show when={logError()}>
        {(e) => <ErrorBanner error={{ ...e(), retryable: true }} retry={() => void loadLog(commits().length === 0)} />}
      </Show>
      <Show when={!logLoading() && !logError() && commits().length === 0}>
        <EmptyState compact icon={<GitCommitHorizontal size={22} />} title="No commits yet" body="Commit something and it shows up here." testId="git-no-commits" />
      </Show>
      <For each={commits()}>
        {(c) => (
          <div
            role="button"
            tabIndex={0}
            class="git-commit-row"
            data-testid="git-commit-row"
            data-git-commit={c.hash}
            data-selected={(() => { const s = selected(); return s?.kind === 'commit' && s.hash === c.hash ? '' : undefined; })()}
            onClick={() => setSelected({ kind: 'commit', hash: c.hash })}
            onKeyDown={(e) => {
              if (!desktopShell() && e.key === 'Enter') setSelected({ kind: 'commit', hash: c.hash });
            }}
          >
            <span class="git-commit-subject truncate">{c.subject || '(no message)'}</span>
            <span class="git-commit-meta">
              <span class="git-hash">{c.short}</span>
              <span class="truncate">{c.author}</span>
              <span class="git-when">{relativeTime(c.time)}</span>
            </span>
            <Show when={refPills(c.refs).length > 0}>
              <span class="git-refs">
                <For each={refPills(c.refs)}>{(r) => <span class="git-ref" data-kind={r.kind}>{r.label}</span>}</For>
              </span>
            </Show>
          </div>
        )}
      </For>
      <Show when={logMore()}>
        <button type="button" class="btn btn-secondary btn-sm git-more" disabled={logLoading()} onClick={() => void loadLog(false)}>
          {logLoading() ? 'Loading…' : 'Load more'}
        </button>
      </Show>
      <Show when={logLoading() && commits().length === 0}>
        <p class="git-note">Loading history…</p>
      </Show>
    </div>
  );

  // Keyed by identity, so a refresh (new change object, same file) updates the open
  // diff in place instead of remounting it.
  const selKey = createMemo(() => {
    const sel = selected();
    return sel ? (sel.kind === 'file' ? `f:${changeKey(sel.change)}` : `c:${sel.hash}`) : undefined;
  });
  const selChange = () => {
    const sel = selected();
    return sel?.kind === 'file' ? sel.change : undefined;
  };
  const detail = () => (
    <Show when={selKey()} keyed>
      {(key) =>
        key.startsWith('f:') ? (
          <GitDiffDetail
            hostId={hostId}
            repo={repoPath()}
            change={selChange()!}
            tick={diffTick()}
            busy={!!busy()}
            onClose={() => setSelected(undefined)}
            onToggle={() => toggleStage(selChange()!)}
            onDiscard={() => setDiscard([selChange()!])}
            onOpen={() => openFile(selChange()!)}
          />
        ) : (
          <GitCommitDetail hostId={hostId} repo={repoPath()} hash={key.slice(2)} onClose={() => setSelected(undefined)} />
        )
      }
    </Show>
  );

  const body = (): JSX.Element => {
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
          body={`${props.target.path} isn't inside a git repo.`}
          testId="git-not-repo"
          action={<button type="button" class="btn btn-secondary" onClick={close}>Back</button>}
        />
      );
    return (
      <div class="git-split" data-detail={detailOpen() ? 'open' : undefined}>
        <div class="git-list-pane">
          <div class="git-tabs-row">
            <div class="seg git-tabs" role="tablist" aria-label="Git view">
              <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'changes'} aria-checked={tab() === 'changes'} onClick={() => { setTab('changes'); setSelected(undefined); }}>
                Changes <Show when={(status()?.files.length ?? 0) > 0}><span class="git-tab-count">{status()?.files.length}</span></Show>
              </button>
              <button type="button" role="tab" class="seg-option" aria-selected={tab() === 'history'} aria-checked={tab() === 'history'} onClick={() => { setTab('history'); setSelected(undefined); }}>
                History
              </button>
            </div>
          </div>
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
          <Show when={tab() === 'changes'} fallback={historyList()}>
            {changesList()}
            {composer}
          </Show>
        </div>
        <Show when={desktopShell()}>
          <div class="git-detail-pane">
            <Show when={detailOpen()} fallback={<EmptyState compact icon={tab() === 'changes' ? <GitBranch size={22} /> : <GitCommitHorizontal size={22} />} title={tab() === 'changes' ? 'Pick a file' : 'Pick a commit'} body={tab() === 'changes' ? '↑/↓ to move, Space to stage, Ctrl+Enter to commit.' : 'See its files and diff here.'} testId="git-detail-placeholder" />}>
              {detail()}
            </Show>
          </div>
        </Show>
      </div>
    );
  };

  return (
    <div class="git-view" role="dialog" aria-label="Git" data-testid="git-view">
      {head}
      {body()}
      <Show when={!desktopShell() && detailOpen()}>
        <div class="git-detail-layer" data-testid="git-detail-layer">{detail()}</div>
      </Show>

      <Show when={headMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} alignEnd label="Git menu" width={250} onDismiss={() => setHeadMenu(undefined)}>
            <MenuRow icon={<GitBranch size={18} />} label="Switch branch…" onSelect={() => { setHeadMenu(undefined); setBranchSheet(true); }} />
            <MenuRow icon={<CloudDownload size={18} />} label="Fetch" disabled={!status()?.remotes.length} onSelect={() => { setHeadMenu(undefined); sync('fetch'); }} />
            <MenuRow icon={<ArrowDownToLine size={18} />} label="Pull with rebase" disabled={!status()?.upstream} onSelect={() => { setHeadMenu(undefined); void act('pull', () => git.pull(hostId, repoPath(), true), () => showToast('Pulled (rebase)')); }} />
            <MenuSeparator />
            <MenuRow icon={<Archive size={18} />} label="Stash changes" disabled={!status()?.files.length} onSelect={() => { setHeadMenu(undefined); void act('stash', () => git.stashPush(hostId, repoPath()), () => showToast('Changes stashed')); }} />
            <MenuRow
              icon={<ArchiveRestore size={18} />}
              label="Pop stash"
              hint={(status()?.stashes ?? 0) > 0 ? <span class="board-menu-note">{status()?.stashes}</span> : undefined}
              disabled={!status()?.stashes}
              onSelect={() => { setHeadMenu(undefined); void act('stash', () => git.stashPop(hostId, repoPath()), () => showToast('Stash applied')); }}
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
              onSelect={() => { const c = m().change; setRowMenu(undefined); toggleStage(c); }}
            />
            <MenuRow icon={<ExternalLink size={18} />} label="Open in editor" disabled={m().change.status === 'D'} onSelect={() => { const c = m().change; setRowMenu(undefined); openFile(c); }} />
            <MenuRow icon={<Copy size={18} />} label="Copy path" onSelect={() => { const c = m().change; setRowMenu(undefined); copyPath(c); }} />
            <Show when={m().change.area !== 'staged'}>
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
          showToast(created ? `Created and switched to ${name}` : `Switched to ${name}`);
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
              void act('discard', () => git.discard(hostId, repoPath(), list.map((c) => c.path)), () => showToast(list[0]!.area === 'untracked' ? 'File deleted' : 'Changes discarded'));
            }}
          >
            {discard()?.[0]?.area === 'untracked' ? 'Delete' : 'Discard'}
          </button>
        </div>
      </BoardSheet>

      <Show when={busy() && ['pull', 'push', 'fetch'].includes(busy()!)}>
        <div class="git-toast" role="status" data-testid="git-progress">
          <RefreshCw size={14} class="refresh-spinning" /> {busy() === 'pull' ? 'Pulling…' : busy() === 'push' ? 'Pushing…' : 'Fetching…'}
        </div>
      </Show>
      <Show when={!busy() && toast()}>
        <div class="git-toast" role="status" data-testid="git-toast">{toast()}</div>
      </Show>
    </div>
  );
}

/** One file's diff (staged vs HEAD, worktree vs index, or a whole untracked file). */
function GitDiffDetail(props: {
  hostId: string;
  repo: string;
  change: GitFileChange;
  tick: number;
  busy: boolean;
  onClose: () => void;
  onToggle: () => void;
  onDiscard: () => void;
  onOpen: () => void;
}) {
  const [state, setState] = createSignal<{ html: string; truncated: boolean; binary: boolean; empty: boolean } | { error: AppError } | undefined>(undefined);
  let shownKey = '';
  onMount(() => {
    if (!desktopShell()) onCleanup(pushBack(props.onClose));
  });
  createEffect(
    on(
      () => [props.change ? changeKey(props.change) : '', props.tick, props.repo] as const,
      async () => {
        const change = props.change;
        if (!change) return;
        // A refresh of the same file keeps the old diff on screen until the new one lands.
        if (shownKey !== changeKey(change)) setState(undefined);
        shownKey = changeKey(change);
        const result = await git.diff(props.hostId, props.repo, change.path, change.area === 'staged', change.area === 'untracked');
        if (!props.change || changeKey(change) !== changeKey(props.change)) return;
        if (!result.ok) {
          setState({ error: result.error });
          return;
        }
        const parsed = parseUnifiedDiff(result.value.text);
        const empty = parsed.files.every((f) => f.hunks.length === 0);
        setState({ html: empty ? '' : renderDiffHtml(parsed), truncated: result.value.truncated, binary: result.value.binary, empty });
      },
    ),
  );
  const parts = () => splitPath(props.change.path);
  const areaLabel = () => (props.change.area === 'staged' ? 'Staged' : props.change.area === 'untracked' ? 'Untracked' : props.change.area === 'conflict' ? 'Conflict' : 'Changes');
  return (
    <div class="git-detail" data-testid="git-diff">
      <header class="git-detail-head detail-head">
        <button type="button" class="ghost-icon-button" aria-label="Close diff" onClick={props.onClose}>
          {desktopShell() ? <X size={20} /> : <ArrowLeft size={22} />}
        </button>
        <div class="git-head-title">
          <span class="git-head-name git-mono truncate" title={props.change.path}>{parts().name}</span>
          <span class="git-head-sub truncate">
            {areaLabel()}
            {props.change.area === 'untracked' || props.change.area === 'conflict' ? '' : ` · ${statusWord(props.change.status)}`}
            {parts().dir ? ` · ${parts().dir}` : ''}
          </span>
        </div>
        <Stat add={props.change.add} del={props.change.del} binary={props.change.binary} />
        <Show when={props.change.area !== 'staged'}>
          <button type="button" class="ghost-icon-button" aria-label={props.change.area === 'untracked' ? 'Delete file' : 'Discard changes'} title={props.change.area === 'untracked' ? 'Delete file' : 'Discard changes'} disabled={props.busy} onClick={props.onDiscard}>
            <Undo2 size={19} />
          </button>
        </Show>
        <button type="button" class="ghost-icon-button" aria-label="Open in editor" title="Open in editor" disabled={props.change.status === 'D'} onClick={props.onOpen}>
          <ExternalLink size={19} />
        </button>
        <button type="button" class="btn btn-secondary btn-sm git-detail-stage" disabled={props.busy} onClick={props.onToggle}>
          {props.change.area === 'staged' ? <Minus size={16} /> : <Plus size={16} />}
          {props.change.area === 'staged' ? 'Unstage' : props.change.area === 'conflict' ? 'Resolved' : 'Stage'}
        </button>
      </header>
      <div class="git-detail-body">
        <Show when={state()} fallback={<p class="git-note">Loading diff…</p>}>
          {(s) => {
            const v = s();
            if ('error' in v) return <ErrorBanner error={{ ...v.error, message: gitErrorText(v.error).body }} />;
            if (v.binary) return <EmptyState compact icon={<GitBranch size={22} />} title="Binary file" body="No line diff for this file." testId="git-diff-binary" />;
            if (v.empty) return <EmptyState compact icon={<Check size={22} />} title="No text changes" body="Only the file mode or name changed." testId="git-diff-empty" />;
            return (
              <>
                <div class="tool-diff diffview-wrap git-diff" innerHTML={v.html} />
                <Show when={v.truncated}>
                  <p class="git-note">The diff is big — showing the first 256 KB.</p>
                </Show>
              </>
            );
          }}
        </Show>
      </div>
    </div>
  );
}

/** One commit: message, files with +/-, and its whole diff. */
function GitCommitDetail(props: { hostId: string; repo: string; hash: string; onClose: () => void }) {
  const [data, setData] = createSignal<GitShowData | { error: AppError } | undefined>(undefined);
  onMount(() => {
    if (!desktopShell()) onCleanup(pushBack(props.onClose));
  });
  createEffect(
    on(
      () => props.hash,
      async (hash) => {
        setData(undefined);
        const result = await git.show(props.hostId, props.repo, hash);
        if (hash !== props.hash) return;
        setData(result.ok ? result.value : { error: result.error });
      },
    ),
  );
  const html = createMemo(() => {
    const d = data();
    return d && !('error' in d) && d.diff ? renderDiffHtml(parseUnifiedDiff(d.diff)) : '';
  });
  const show = () => {
    const d = data();
    return d && !('error' in d) ? d : undefined;
  };
  return (
    <div class="git-detail" data-testid="git-commit">
      <header class="git-detail-head detail-head">
        <button type="button" class="ghost-icon-button" aria-label="Close commit" onClick={props.onClose}>
          {desktopShell() ? <X size={20} /> : <ArrowLeft size={22} />}
        </button>
        <div class="git-head-title">
          <span class="git-head-name truncate">{show()?.commit.subject ?? props.hash.slice(0, 7)}</span>
          <span class="git-head-sub truncate">
            <Show when={show()} fallback="Loading…">
              {(s) => `${s().commit.short} · ${s().commit.author} · ${relativeTime(s().commit.time)}`}
            </Show>
          </span>
        </div>
        <button
          type="button"
          class="ghost-icon-button"
          aria-label="Copy commit hash"
          title="Copy hash"
          onClick={() => void navigator.clipboard?.writeText(props.hash)}
        >
          <Copy size={18} />
        </button>
      </header>
      <div class="git-detail-body">
        <Show when={data()} fallback={<p class="git-note">Loading commit…</p>}>
          {(d) => {
            const v = d();
            if ('error' in v) return <ErrorBanner error={{ ...v.error, message: gitErrorText(v.error).body }} />;
            return (
              <>
                <Show when={refPills(v.commit.refs).length > 0}>
                  <div class="git-refs git-refs-detail">
                    <For each={refPills(v.commit.refs)}>{(r) => <span class="git-ref" data-kind={r.kind}>{r.label}</span>}</For>
                  </div>
                </Show>
                <Show when={v.body}>
                  <pre class="git-commit-body">{v.body}</pre>
                </Show>
                <div class="section-row">
                  <span class="section-label">
                    Files <span class="git-group-count">{v.files.length}</span>
                  </span>
                  <Stat add={v.files.reduce((n, f) => n + (f.add ?? 0), 0)} del={v.files.reduce((n, f) => n + (f.del ?? 0), 0)} />
                </div>
                <div class="git-show-files">
                  <For each={v.files}>
                    {(f) => (
                      <div class="git-show-file">
                        <span class="files-git-badge git-badge" data-git={f.status}>{f.status}</span>
                        <span class="git-mono truncate git-show-path" title={f.path}>{f.path}</span>
                        <Stat add={f.add} del={f.del} binary={f.add === undefined} />
                      </div>
                    )}
                  </For>
                </div>
                <Show when={html()}>
                  <div class="tool-diff diffview-wrap git-diff" innerHTML={html()} />
                </Show>
                <Show when={v.truncated}>
                  <p class="git-note">The diff is big — showing the first 256 KB.</p>
                </Show>
              </>
            );
          }}
        </Show>
      </div>
    </div>
  );
}

/** Branch switcher: local first (current checked), remotes, and "new branch". */
function BranchSheet(props: {
  open: boolean;
  hostId: string;
  repo: string;
  current?: string;
  onClose: () => void;
  onSwitched: (name: string, created: boolean) => void;
}) {
  const [list, setList] = createSignal<Branch[] | undefined>(undefined);
  const [error, setError] = createSignal<AppError | undefined>(undefined);
  const [name, setName] = createSignal('');
  const [busy, setBusy] = createSignal<string | undefined>(undefined);
  const [filter, setFilter] = createSignal('');
  createEffect(
    on(
      () => props.open,
      async (open) => {
        if (!open) return;
        setList(undefined);
        setError(undefined);
        setName('');
        setFilter('');
        const result = await git.branches(props.hostId, props.repo);
        if (result.ok) setList(result.value.branches);
        else setError(result.error);
      },
    ),
  );
  const visible = createMemo(() => {
    const q = filter().trim().toLowerCase();
    return (list() ?? []).filter((b) => !q || b.name.toLowerCase().includes(q));
  });
  async function pick(branch: string, create: boolean) {
    if (busy()) return;
    setBusy(branch);
    setError(undefined);
    const result = await git.switch(props.hostId, props.repo, branch, create);
    setBusy(undefined);
    if (result.ok) props.onSwitched(branch, create);
    else setError(result.error);
  }
  const row = (b: Branch) => (
    <button
      type="button"
      class="git-branch-row"
      data-testid="git-branch-row"
      data-current={b.current ? '' : undefined}
      disabled={!!busy() || b.current}
      onClick={() => void pick(b.name, false)}
    >
      <span class="git-branch-icon">{b.current ? <Check size={17} /> : <GitBranch size={17} />}</span>
      <span class="git-row-main">
        <span class="git-row-name truncate">{b.name}</span>
        <span class="git-row-dir truncate">
          {[b.time ? relativeTime(b.time) : '', b.subject].filter(Boolean).join(' · ')}
        </span>
      </span>
      <Show when={b.ahead || b.behind}>
        <span class="git-branch-track">{aheadBehind(b)}</span>
      </Show>
      <Show when={busy() === b.name}>
        <RefreshCw size={15} class="refresh-spinning" />
      </Show>
    </button>
  );
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Branches" testId="git-branch-sheet">
      <form
        class="git-new-branch"
        onSubmit={(e) => {
          e.preventDefault();
          const n = name().trim();
          if (n) void pick(n, true);
        }}
      >
        <input
          type="text"
          class="files-search-input"
          placeholder="New branch name"
          aria-label="New branch name"
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
          autocapitalize="off"
          autocorrect="off"
          spellcheck={false}
        />
        <button type="submit" class="btn btn-primary btn-sm" disabled={!name().trim() || !!busy()}>
          <Plus size={16} /> Create
        </button>
      </form>
      <Show when={error()}>
        {(e) => {
          const t = gitErrorText(e());
          return <div class="git-error"><ErrorBanner error={{ code: e().code, message: t.body, hint: t.hint, retryable: false }} dismiss={() => setError(undefined)} /></div>;
        }}
      </Show>
      <Show when={(list()?.length ?? 0) > 8}>
        <input type="search" class="files-search-input git-branch-filter" placeholder="Filter branches" aria-label="Filter branches" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} />
      </Show>
      <Show when={list()} fallback={<Show when={!error()}><p class="git-note">Loading branches…</p></Show>}>
        <div class="git-branch-list">
          <Show when={visible().some((b) => !b.remote)}>
            <div class="section-label">Local</div>
            <For each={visible().filter((b) => !b.remote)}>{row}</For>
          </Show>
          <Show when={visible().some((b) => b.remote)}>
            <div class="section-label">Remote</div>
            <For each={visible().filter((b) => b.remote)}>{row}</For>
          </Show>
          <Show when={visible().length === 0}>
            <p class="git-note">{filter() ? 'No branch matches.' : 'No branches yet — commit first.'}</p>
          </Show>
        </div>
      </Show>
      <Show when={props.current}>
        <p class="git-note git-branch-foot">
          On <strong class="git-mono">{props.current}</strong>. Switching keeps your uncommitted changes when git can.
        </p>
      </Show>
    </BoardSheet>
  );
}
