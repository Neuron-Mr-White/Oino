/**
 * The Git pane's History tab (WP-A15, moved out of the old GitView): the
 * `git log` list with ref pills, virtualized like the changes list, paged
 * 50 at a time ("Load more"). No commit graph (Z-99: skipped).
 */
import { For, Show, type JSX } from 'solid-js';
import { createVirtualizer } from '@tanstack/solid-virtual';
import { GitCommitHorizontal } from 'lucide-solid';
import { refPills, relativeTime, type GitCommitInfo } from '../../lib/git';
import { desktopShell } from '../../lib/layout';
import { EmptyState } from '../EmptyState';

export function GitHistory(props: {
  commits: GitCommitInfo[];
  selected?: string;
  more: boolean;
  loading: boolean;
  onSelect: (hash: string) => void;
  onMore: () => void;
  footer?: JSX.Element;
}) {
  let scroll: HTMLDivElement | null = null;
  const virtualizer = createVirtualizer({
    get count() {
      return props.commits.length;
    },
    getScrollElement: () => scroll,
    estimateSize: () => (desktopShell() ? 56 : 64),
    overscan: 10,
  });
  return (
    <div
      class="git-list-scroll git-virtual"
      data-testid="git-history"
      ref={(el) => {
        scroll = el;
      }}
    >
      <Show when={props.commits.length === 0 && !props.loading}>
        <EmptyState compact icon={<GitCommitHorizontal size={22} />} title="No commits yet" body="Commit something and it shows up here." testId="git-no-commits" />
      </Show>
      <div class="git-virtual-inner" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        <For each={virtualizer.getVirtualItems()}>
          {(v) => {
            const c = () => props.commits[v.index];
            return (
              <Show when={c()} keyed>
                {(commit) => (
                  <div
                    ref={(el: HTMLElement) => {
                      el.style.transform = `translateY(${v.start}px)`;
                      virtualizer.measureElement(el);
                    }}
                    data-index={v.index}
                    class="git-commit-row git-vrow"
                    data-testid="git-commit-row"
                    data-git-commit={commit.hash}
                    data-selected={props.selected === commit.hash ? '' : undefined}
                    role="button"
                    tabIndex={0}
                    onClick={() => props.onSelect(commit.hash)}
                    onKeyDown={(e) => {
                      if (!desktopShell() && e.key === 'Enter') props.onSelect(commit.hash);
                    }}
                  >
                    <span class="git-commit-subject truncate">{commit.subject || '(no message)'}</span>
                    <span class="git-commit-meta">
                      <span class="git-hash">{commit.short}</span>
                      <span class="truncate">{commit.author}</span>
                      <span class="git-when">{relativeTime(commit.time)}</span>
                    </span>
                    <Show when={refPills(commit.refs).length > 0}>
                      <span class="git-refs">
                        <For each={refPills(commit.refs)}>{(r) => <span class="git-ref" data-kind={r.kind}>{r.label}</span>}</For>
                      </span>
                    </Show>
                  </div>
                )}
              </Show>
            );
          }}
        </For>
      </div>
      <Show when={props.more}>
        <button type="button" class="btn btn-secondary btn-sm git-more" disabled={props.loading} onClick={() => props.onMore()}>
          {props.loading ? 'Loading…' : 'Load more'}
        </button>
      </Show>
      <Show when={props.loading && props.commits.length === 0}>
        <p class="git-note">Loading history…</p>
      </Show>
      {props.footer}
    </div>
  );
}
