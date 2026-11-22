/**
 * Branch switcher (moved out of the old GitView, WP-A15): local first
 * (current checked), remotes, and "new branch". Kept whole (Z-97: unipi's
 * full client actions live inside the pane).
 */
import { createEffect, createMemo, createSignal, on, Show, For } from 'solid-js';
import { Check, GitBranch, Plus, RefreshCw } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { aheadBehind, git, gitErrorText, relativeTime, type GitBranch as Branch } from '../../lib/git';
import { ErrorBanner } from '../ErrorBanner';
import { BoardSheet } from '../board/BoardSheet';

export function BranchSheet(props: {
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
