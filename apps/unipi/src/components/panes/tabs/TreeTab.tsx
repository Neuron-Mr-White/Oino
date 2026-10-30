/**
 * The `tree` side-pane tab (WP-A10, Z-18): the TreeSheet port — every branch
 * of the session (pi's `/tree`), the five pi filters, "Go to this point",
 * "Start a new branch here" (with the "Summarize the branch I leave" option)
 * and the jump restoring the message into the composer. A sheet on the phone
 * (§3.9); the same component renders in both.
 */
import { createEffect, createMemo, createSignal, For, Show } from 'solid-js';
import { Check, CornerDownRight, GitBranch, ListTree } from 'lucide-solid';
import { AnchoredMenu, MenuRow } from '../../board/AnchoredMenu';
import { sessions, type SessionHandle } from '../../../lib/session/store';
import type { SessionEntry } from '../../../lib/session/types';
import { focusComposer, restoreComposer } from '../../../lib/composer/drafts';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { tabBaseOf } from './sessionTabs';
import { buildTreeNodes, filterTree, flattenTree, readTreeFilter, rememberTreeFilter, TREE_FILTERS, TREE_MAX_DEPTH, type TreeNode, type TreeFilter } from './tree';
import './tabs.css';

type WireNode = { entry: SessionEntry; children?: WireNode[]; label?: unknown };
type TreeReply = { tree?: WireNode[]; leafId?: string | null };
type NavigateReply = { cancelled?: boolean; editorText?: string };

/** `navigate_tree`: moves the leaf; `summarize` writes the branch summary. */
async function navigate(session: SessionHandle, targetId: string, summarize: boolean): Promise<NavigateReply> {
  return session.cmd<NavigateReply>({ type: '_oino/navigate_tree', targetId, ...(summarize ? { summarize: true } : {}) });
}

export function TreeTab(props: PaneTabProps) {
  const base = () => tabBaseOf(props.tab.params, { hostId: props.hostId, sessionId: props.sessionId });
  const handle = () => (base().sessionId ? sessions().ensure(base().hostId, base().sessionId) : undefined);
  const [nodes, setNodes] = createSignal<TreeNode[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | undefined>();
  const [filter, setFilter] = createSignal<TreeFilter>(readTreeFilter());
  const [summarize, setSummarize] = createSignal(true);
  const [rowMenu, setRowMenu] = createSignal<{ node: TreeNode; anchor: DOMRect } | null>(null);
  let listEl: HTMLDivElement | undefined;

  const refresh = async () => {
    const s = handle();
    if (!s) return;
    try {
      const reply = await s.cmd<TreeReply>({ type: 'get_tree' });
      setNodes(buildTreeNodes(Array.isArray(reply?.tree) ? reply.tree : [], reply?.leafId ?? null));
      setError(undefined);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };
  // Reload when the stream comes live (a tab can open while the session is
  // still connecting; `cmd` rejects before that) and on a reopen.
  createEffect(() => {
    const s = handle();
    if (s?.state.conn !== 'live') return;
    void refresh();
  });

  const rows = createMemo(() => flattenTree(filterTree(nodes(), filter())));
  createEffect(() => {
    if (loading() || !listEl) return;
    queueMicrotask(() => listEl?.querySelector<HTMLElement>('[data-current]')?.scrollIntoView?.({ block: 'center' }));
  });

  /** The jump itself: move the leaf, restore the message into the composer (Z-18). */
  const goTo = async (node: TreeNode, withSummary: boolean) => {
    const s = handle();
    if (!s) return;
    try {
      const reply = await navigate(s, node.id, withSummary);
      if (reply?.cancelled) {
        setError('The jump was cancelled.');
        return;
      }
      if (typeof reply?.editorText === 'string' && reply.editorText.trim()) {
        restoreComposer(s.key, { text: reply.editorText });
        focusComposer(s.key);
      }
      setLoading(true);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const onRow = (node: TreeNode, anchor: DOMRect) => {
    // On the current branch (not the leaf) the row asks: go there, or branch.
    if (node.onPath && !node.current) return setRowMenu({ node, anchor });
    void goTo(node, summarize());
  };

  // Back is the phone surface's job (`SessionTabLayer`); the desktop tab has
  // no Back of its own, so the view never stacks a second handler.

  return (
    <div class="ttab" data-testid="tree-tab">
      <div class="ttab-filters" role="radiogroup" aria-label="Tree filter">
        <For each={TREE_FILTERS}>
          {(f) => (
            <button type="button" role="radio" class="ttab-filter" aria-checked={filter() === f.id} onClick={() => (setFilter(f.id), rememberTreeFilter(f.id))}>
              {f.label}
            </button>
          )}
        </For>
      </div>
      <div class="ttab-list" ref={listEl}>
        <Show
          when={!loading()}
          fallback={<p class="ttab-note">Loading…</p>}
        >
          <Show
            when={rows().length > 0}
            fallback={
              <p class="ttab-note">
                {error() ? `Couldn't load the tree: ${error()}` : nodes().length === 0 ? 'This session has no entries yet' : 'Nothing matches this filter'}
              </p>
            }
          >
            <For each={rows()}>
              {(row) => {
                const node = row.node;
                const cells = row.cells.slice(-TREE_MAX_DEPTH);
                return (
                  <button
                    type="button"
                    class="ttab-row"
                    data-on-path={node.onPath ? '' : undefined}
                    data-current={node.current ? '' : undefined}
                    data-depth={row.depth}
                    aria-current={node.current ? 'true' : undefined}
                    onClick={(e) => onRow(node, (e.currentTarget as HTMLElement).getBoundingClientRect())}
                  >
                    <Show when={cells.length > 0}>
                      <span class="ttab-gutter shrink-0" aria-hidden="true">
                        <For each={cells}>{(cell) => <span class="ttab-cell" data-cell={cell || undefined} />}</For>
                      </span>
                    </Show>
                    <span class="min-w-0 flex-1 text-left">
                      <span class="block truncate">{node.preview || `(${node.kind})`}</span>
                      <span class="block truncate ttab-sub">
                        {node.kind}
                        {node.label ? ` · ${node.label}` : ''}
                      </span>
                    </span>
                    <Show when={row.depth > TREE_MAX_DEPTH}>
                      <span class="ttab-deep" title={`${row.depth} branches deep`}>↳{row.depth}</span>
                    </Show>
                    <Show when={node.current}>
                      <Check size={16} class="ttab-current shrink-0" />
                    </Show>
                  </button>
                );
              }}
            </For>
          </Show>
        </Show>
      </div>
      <label class="ttab-foot">
        <input type="checkbox" checked={summarize()} onChange={(e) => setSummarize(e.currentTarget.checked)} />
        Summarize the branch I leave
      </label>
      <Show when={rowMenu()} keyed>
        {(m) => (
          <AnchoredMenu anchor={m.anchor} width={260} label="Tree point" onDismiss={() => setRowMenu(null)}>
            <MenuRow icon={<CornerDownRight size={18} />} label="Go to this point" onSelect={() => (setRowMenu(null), void goTo(m.node, false))} />
            <MenuRow icon={<GitBranch size={18} />} label="Start a new branch here" onSelect={() => (setRowMenu(null), void goTo(m.node, summarize()))} />
          </AnchoredMenu>
        )}
      </Show>
    </div>
  );
}

setPaneComponent('tree', TreeTab);

/** The tab's empty header icon row (used by the phone sheet chrome). */
export const TreeTabIcon = ListTree;
