/**
 * Git detail views (WP-A15, moved out of the old GitView): one file's diff
 * (fetched, or preloaded by the Branch / Last-turn sources) and one commit.
 * The pane mounts diffs inline (Z-96); the phone layer mounts them
 * full-screen with a back button. Part of the status pane's Git section.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, For } from 'solid-js';
import {
  ArrowLeft,
  Check,
  Copy,
  ExternalLink,
  GitBranch,
  Minus,
  Plus,
  Undo2,
  X,
} from 'lucide-solid';
import type { AppError, GitDiffData } from '@unipi/protocol';
import { pushBack } from '../../lib/back';
import { desktopShell } from '../../lib/layout';
import { parseUnifiedDiff, renderDiffHtml } from '../../lib/markdown';
import { changeKey, git, gitErrorText, relativeTime, refPills, splitPath, statusWord, type GitFileChange, type GitShowData } from '../../lib/git';
import { EmptyState } from '../EmptyState';
import { ErrorBanner } from '../ErrorBanner';

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

type DiffState = { html: string; truncated: boolean; binary: boolean; empty: boolean } | { error: AppError } | undefined;

/** The shared diff body (inline and full-screen both). */
function DiffBody(props: { state: DiffState }) {
  return (
    <Show when={props.state} fallback={<p class="git-note">Loading diff…</p>}>
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
  );
}

/** One file's diff: fetched lazily (Unstaged/Staged) or from the row
 *  (Branch / Last turn already carry it). `inline` = inside the pane's list. */
export function GitDiffDetail(props: {
  hostId: string;
  repo: string;
  change: GitFileChange;
  /** Refetch tick (a refresh happened). */
  tick?: number;
  /** The row's own diff (branch / last-turn sources); no fetch. */
  preloaded?: GitDiffData;
  /** Area wording override for preloaded rows ("vs origin/main"). */
  areaLabel?: string;
  inline?: boolean;
  busy?: boolean;
  onClose: () => void;
  onToggle?: () => void;
  onDiscard?: () => void;
  onOpen?: () => void;
  /** The diff body settled (the inline list remeasures its row height). */
  onSettled?: () => void;
}) {
  const [state, setState] = createSignal<DiffState>(undefined);
  let shownKey = '';
  const full = () => !props.inline || !desktopShell();
  onMount(() => {
    if (full()) onCleanup(pushBack(props.onClose));
  });
  createEffect(
    on(
      () => [props.change ? changeKey(props.change) : '', props.tick ?? 0, props.repo, props.preloaded] as const,
      () => {
        const change = props.change;
        if (!change) return;
        const key = changeKey(change);
        // A refresh of the same file keeps the old diff on screen until the new one lands.
        if (shownKey !== key) setState(undefined);
        shownKey = key;
        const pre = props.preloaded;
        if (pre) {
          const parsed = parseUnifiedDiff(pre.text);
          const empty = parsed.files.every((f) => f.hunks.length === 0);
          setState({ html: empty ? '' : renderDiffHtml(parsed), truncated: pre.truncated, binary: pre.binary, empty });
          props.onSettled?.();
          return;
        }
        void git.diff(props.hostId, props.repo, change.path, change.area === 'staged', change.area === 'untracked').then((result) => {
          if (!props.change || changeKey(change) !== changeKey(props.change)) return;
          if (!result.ok) {
            setState({ error: result.error });
            return;
          }
          const parsed = parseUnifiedDiff(result.value.text);
          const empty = parsed.files.every((f) => f.hunks.length === 0);
          setState({ html: empty ? '' : renderDiffHtml(parsed), truncated: result.value.truncated, binary: result.value.binary, empty });
          props.onSettled?.();
        });
      },
    ),
  );
  const parts = () => splitPath(props.change.path);
  const areaLabel = () => props.areaLabel ?? (props.change.area === 'staged' ? 'Staged' : props.change.area === 'untracked' ? 'Untracked' : props.change.area === 'conflict' ? 'Conflict' : 'Changes');
  const head = (
    <header class="git-detail-head detail-head" data-inline={props.inline ? '' : undefined}>
      <button type="button" class="ghost-icon-button" aria-label="Close diff" onClick={props.onClose}>
        {full() ? <ArrowLeft size={22} /> : <X size={18} />}
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
      <Show when={!props.inline && props.change.area !== 'staged' && props.onDiscard}>
        <button type="button" class="ghost-icon-button" aria-label={props.change.area === 'untracked' ? 'Delete file' : 'Discard changes'} title={props.change.area === 'untracked' ? 'Delete file' : 'Discard changes'} disabled={props.busy} onClick={props.onDiscard}>
          <Undo2 size={19} />
        </button>
      </Show>
      <Show when={props.onOpen}>
        <button type="button" class="ghost-icon-button" aria-label="Open in editor" title="Open in editor" disabled={props.change.status === 'D'} onClick={props.onOpen}>
          <ExternalLink size={19} />
        </button>
      </Show>
      <Show when={props.onToggle}>
        <button type="button" class="btn btn-secondary btn-sm git-detail-stage" disabled={props.busy} onClick={props.onToggle}>
          {props.change.area === 'staged' ? <Minus size={16} /> : <Plus size={16} />}
          {props.change.area === 'staged' ? 'Unstage' : props.change.area === 'conflict' ? 'Resolved' : 'Stage'}
        </button>
      </Show>
    </header>
  );
  return (
    <div class="git-detail" data-testid="git-diff" data-inline={props.inline ? '' : undefined}>
      {head}
      <div class="git-detail-body">
        <DiffBody state={state()} />
      </div>
    </div>
  );
}

/** One commit: message, files with +/-, and its whole diff. */
export function GitCommitDetail(props: { hostId: string; repo: string; hash: string; onClose: () => void }) {
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
                  <Stat add={v.files.reduce((n: number, f) => n + (f.add ?? 0), 0)} del={v.files.reduce((n: number, f) => n + (f.del ?? 0), 0)} />
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
