/**
 * The Git pane's changes list (WP-A15): the flattened group/file rows of the
 * current review source, virtualized (`@tanstack/solid-virtual`) so 2 000
 * changed files scroll at 60 fps (Z-96), with the selected file's diff
 * expanded inline under its row (lazy per-file diff; the Branch and
 * Last-turn sources pass the row's own diff along).
 */
import { createEffect, createMemo, on, onCleanup, For, Show, type JSX } from 'solid-js';
import { createVirtualizer } from '@tanstack/solid-virtual';
import { Minus, Plus, RefreshCw } from 'lucide-solid';
import type { GitDiffData } from '@unipi/protocol';
import { changeKey, splitPath, statusWord, totals, type GitFileChange, type GitRow } from '../../lib/git';
import { desktopShell } from '../../lib/layout';
import { Stat, GitDiffDetail } from './GitDetail';

type Item =
  | { t: 'group'; row: Extract<GitRow, { kind: 'group' }> }
  | { t: 'file'; row: Extract<GitRow, { kind: 'file' }> }
  | { t: 'diff'; key: string; row: Extract<GitRow, { kind: 'file' }> };

export function GitChanges(props: {
  rows: GitRow[];
  /** The inline-expanded diff's diffKey (desktop pane; the phone opens the detail layer). */
  expandedKey?: string;
  selectedKey?: string;
  hostId: string;
  repo: string;
  /** Refetch tick for lazily fetched diffs. */
  tick: number;
  /** Rows that already carry their diff (branch / last-turn sources). */
  preloaded?: (key: string) => GitDiffData | undefined;
  /** Area wording for preloaded rows ("vs origin/main"). */
  sourceLabel?: string;
  busy?: string | undefined;
  canStage: boolean;
  onExpand: (key: string | undefined) => void;
  onSelect: (key: string | undefined) => void;
  onToggleStage: (change: GitFileChange) => void;
  onStageGroup?: (key: string, files: GitFileChange[]) => void;
  onRowMenu: (change: GitFileChange, anchor: DOMRect) => void;
  children?: JSX.Element;
}) {
  let scroll: HTMLDivElement | null = null;
  /** Inline diff rows by diff key: re-measured when the lazy diff body settles. */
  const diffRows = new Map<string, HTMLElement>();
  onCleanup(() => diffRows.clear());

  const items = createMemo<Item[]>(() => {
    const out: Item[] = [];
    for (const row of props.rows) {
      if (row.kind === 'group') {
        out.push({ t: 'group', row });
        continue;
      }
      out.push({ t: 'file', row });
      if (row.diffKey === props.expandedKey && desktopShell()) out.push({ t: 'diff', key: row.diffKey, row });
    }
    return out;
  });

  const virtualizer = createVirtualizer({
    get count() {
      return items().length;
    },
    getScrollElement: () => scroll,
    estimateSize: (i) => {
      const item = items()[i];
      if (!item) return 48;
      if (item.t === 'group') return 34;
      if (item.t === 'diff') return 320;
      return desktopShell() ? 44 : 52;
    },
    overscan: 8,
    getItemKey: (i) => {
      const item = items()[i]!;
      return item.t === 'diff' ? item.key : item.row.key;
    },
  });

  /** A diff body grew after its lazy fetch: re-measure that row (not the list,
   *  which would fall back to the 320 px estimate and leave a gap). */
  const remeasure = (key: string) =>
    requestAnimationFrame(() => {
      const el = diffRows.get(key);
      if (el) virtualizer.measureElement(el);
    });

  // Follow the keyboard selection (↑/↓): centre the row in the window.
  createEffect(
    on(
      () => props.selectedKey,
      (key) => {
        if (!key) return;
        const at = items().findIndex((it) => it.t === 'file' && it.row.key === key);
        if (at > 0) virtualizer.scrollToIndex(at, { align: 'center' });
      },
      { defer: true },
    ),
  );

  return (
    <div
      class="git-list-scroll git-virtual"
      data-testid="git-changes"
      ref={(el) => {
        scroll = el;
      }}
    >
      <div class="git-virtual-inner" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        <For each={virtualizer.getVirtualItems()}>
          {(v) => {
            const item = () => items()[v.index]!;
            return (
              <Show when={item()} keyed>
                {(it) => {
                  const rowRef = (el: HTMLElement) => {
                    el.style.transform = `translateY(${v.start}px)`;
                    virtualizer.measureElement(el);
                  };
                  if (it.t === 'group') {
                    const group = it.row;
                    return (
                      <div class="git-vrow git-group-head" ref={rowRef} data-index={v.index}>
                        <span class="section-label">
                          {group.label} <span class="git-group-count">{group.files.length}</span>
                        </span>
                        <span class="git-group-tools">
                          <Stat {...totals(group.files)} />
                          <Show when={props.canStage && props.onStageGroup && group.key !== 'g:conflict' && group.key !== 'g:branch' && group.key !== 'g:turn'}>
                            <button
                              type="button"
                              class="git-link"
                              disabled={!!props.busy}
                              onClick={() => props.onStageGroup?.(group.key, group.files)}
                            >
                              {group.key === 'g:staged' ? 'Unstage all' : 'Stage all'}
                            </button>
                          </Show>
                        </span>
                      </div>
                    );
                  }
                  if (it.t === 'diff') {
                    return (
                      <div
                        class="git-vrow git-inline-diff"
                        ref={(el) => {
                          diffRows.set(it.key, el);
                          rowRef(el);
                        }}
                        data-index={v.index}
                      >
                        <GitDiffDetail
                          hostId={props.hostId}
                          repo={props.repo}
                          change={it.row.change}
                          tick={props.tick}
                          preloaded={props.preloaded?.(it.row.diffKey)}
                          areaLabel={props.preloaded?.(it.row.diffKey) ? props.sourceLabel : undefined}
                          onSettled={() => remeasure(it.key)}
                          inline
                          onClose={() => props.onExpand(undefined)}
                          onToggle={props.canStage ? () => props.onToggleStage(it.row.change) : undefined}
                        />
                      </div>
                    );
                  }
                  const change = it.row.change;
                  const parts = () => splitPath(change.path);
                  const isSelected = () => props.selectedKey === it.row.key;
                  const isOpen = () => props.expandedKey === it.row.diffKey;
                  return (
                    <div
                      ref={rowRef}
                      data-index={v.index}
                      role="button"
                      tabIndex={0}
                      class="git-row git-vrow"
                      data-testid="git-row"
                      data-git-key={changeKey(change)}
                      data-selected={isSelected() || isOpen() ? '' : undefined}
                      aria-label={`${change.path}, ${statusWord(change.status)}`}
                      onClick={() => {
                        props.onSelect(it.row.key);
                        if (desktopShell()) props.onExpand(isOpen() ? undefined : it.row.diffKey);
                      }}
                      onKeyDown={(e) => {
                        if (desktopShell()) return;
                        if (e.key === 'Enter') props.onSelect(it.row.key);
                      }}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        props.onRowMenu(change, (e.currentTarget as HTMLElement).getBoundingClientRect());
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
                      <Show when={props.canStage}>
                        <button
                          type="button"
                          class="ghost-icon-button git-stage-btn"
                          data-staged={change.area === 'staged' ? '' : undefined}
                          aria-label={change.area === 'staged' ? `Unstage ${change.path}` : change.area === 'conflict' ? `Mark ${change.path} resolved` : `Stage ${change.path}`}
                          disabled={!!props.busy}
                          onClick={(e) => {
                            e.stopPropagation();
                            props.onToggleStage(change);
                          }}
                        >
                          <Show when={props.busy === `stage:${change.path}`} fallback={change.area === 'staged' ? <Minus size={18} /> : <Plus size={18} />}>
                            <RefreshCw size={16} class="refresh-spinning" />
                          </Show>
                        </button>
                      </Show>
                    </div>
                  );
                }}
              </Show>
            );
          }}
        </For>
      </div>
      {props.children}
    </div>
  );
}
