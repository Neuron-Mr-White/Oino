/**
 * The `tree` side-pane tab (WP-A10, Z-18): the TreeSheet port — every branch
 * of the session (pi's `/tree`), the five pi filters, "Go to this point",
 * "Start a new branch here" (with the "Summarize the branch I leave" option)
 * and the jump restoring the message into the composer. A sheet on the phone
 * (§3.9); the same component renders in both.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { Check, CornerDownRight, GitBranch, ListTree } from 'lucide-solid';
import { AnchoredMenu, MenuRow } from '../../board/AnchoredMenu';
import { sessions, type NavigateReply, type SessionHandle } from '../../../lib/session/store';
import type { SessionEntry } from '../../../lib/session/types';
import { focusComposer, restoreComposer } from '../../../lib/composer/drafts';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { tabBaseOf } from './sessionTabs';
import { buildTreeNodes, buildTreeNodesFromEntries, filterTree, flattenTree, readTreeFilter, rememberTreeFilter, TREE_FILTERS, TREE_MAX_DEPTH, treeKindLabel, type TreeNode, type TreeFilter } from './tree';
import './tabs.css';

type WireNode = { entry: SessionEntry; children?: WireNode[]; label?: unknown };
type TreeReply = { tree?: WireNode[]; leafId?: string | null };

/**
 * `navigate_tree`: moves the leaf; `summarize` writes the branch summary. [chat-tree] Through
 * `SessionHandle.navigate`, so the open chat reloads the branch it lands on.
 */
async function navigate(session: SessionHandle, targetId: string, summarize: boolean): Promise<NavigateReply> {
  const opts = summarize ? { summarize: true } : {};
  return session.navigate ? session.navigate(targetId, opts) : session.cmd<NavigateReply>({ type: '_oino/navigate_tree', targetId, ...opts });
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

  // A request that never answers must not leave "Loading…" up forever: every step has a deadline,
  // and the last one's failure is shown with a Retry.
  const LOAD_TIMEOUT_MS = 45_000;
  const STEP_TIMEOUT_MS = 10_000;
  const TREE_HEAD_START_MS = 2_500;
  const PAGE = 500;
  const within = <T,>(work: Promise<T>, ms: number): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const t = setTimeout(() => reject(new Error('The host took too long to answer.')), ms);
      work.then(
        (v) => (clearTimeout(t), resolve(v)),
        (e) => (clearTimeout(t), reject(e)),
      );
    });
  let loadSeq = 0;
  /**
   * `get_tree` nests one level per entry: on a chat thousands of entries long the reply is one
   * chain thousands deep, which the host's JSON layer refuses (recursion limit) or never finishes
   * — the tab then loaded forever. So: the nested tree while it works, else the flat entries
   * (parent links rebuild the tree), else the current branch page by page.
   */
  const load = async (s: SessionHandle): Promise<TreeNode[]> => {
    // The flat entries are asked at once, beside get_tree: on a long real chat get_tree never
    // answers, and waiting out its whole deadline first kept the tab on Loading for 10 s per
    // open (real-app rig, 2559-entry chat). get_tree only gets a short head start.
    const flatReq = s.cmd<{ entries?: SessionEntry[]; leafId?: string | null }>({ type: 'get_entries' });
    flatReq.catch(() => undefined);
    try {
      const reply = await within(s.cmd<TreeReply>({ type: 'get_tree' }), TREE_HEAD_START_MS);
      return buildTreeNodes(Array.isArray(reply?.tree) ? reply.tree : [], reply?.leafId ?? null);
    } catch {
      // fall through to the flat entries
    }
    try {
      const flat = await within(flatReq, STEP_TIMEOUT_MS * 2);
      if (Array.isArray(flat?.entries)) return buildTreeNodesFromEntries(flat.entries, flat.leafId ?? null);
    } catch {
      // fall through to the paged branch
    }
    const all: SessionEntry[] = [];
    let before: string | undefined;
    let leafId: string | null = null;
    for (let i = 0; i < 200; i++) {
      const page = await within(
        s.cmd<{ entries?: SessionEntry[]; leafId?: string | null; hasMore?: boolean; cursor?: string | null }>({ type: 'get_entries', limit: PAGE, ...(before ? { before } : {}) }),
        STEP_TIMEOUT_MS,
      );
      const got = Array.isArray(page?.entries) ? page.entries : [];
      if (i === 0) leafId = page?.leafId ?? null;
      all.unshift(...got);
      if (!page?.hasMore || got.length === 0) break;
      before = page.cursor ?? got[0]!.id;
    }
    return buildTreeNodesFromEntries(all, leafId);
  };
  const refresh = async () => {
    const s = handle();
    if (!s) return;
    const mine = ++loadSeq;
    try {
      const next = await within(load(s), LOAD_TIMEOUT_MS);
      if (mine !== loadSeq) return;
      setNodes(next);
      setError(undefined);
    } catch (e) {
      if (mine !== loadSeq) return;
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mine === loadSeq) setLoading(false);
    }
  };
  const retry = () => {
    setLoading(true);
    setError(undefined);
    void refresh();
  };
  // Reload when the stream comes live (a tab can open while the session is
  // still connecting; `cmd` rejects before that) and on a reopen.
  // [chat-tree] …and whenever the branch moved (a "Return here" in the chat, another window,
  // the phone): `branchRev` bumps once per jump, never per streamed message.
  createEffect(() => {
    const s = handle();
    if (s?.state.conn !== 'live') {
      // Never connected: say so instead of loading forever.
      const t = setTimeout(() => {
        if (!loading()) return;
        setError('The chat is not connected to its host.');
        setLoading(false);
      }, 15_000);
      onCleanup(() => clearTimeout(t));
      return;
    }
    void s.state.branchRev;
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
      // The branch reload bumps `branchRev`, which refreshes the tree (a bare handle: here).
      if (!s.navigate) await refresh();
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
              <Show
                when={error()}
                fallback={<p class="ttab-note">{nodes().length === 0 ? 'This session has no entries yet' : 'Nothing matches this filter'}</p>}
              >
                <div class="ttab-note" role="alert" data-testid="tree-error">
                  <p>Couldn't load the tree: {error()}</p>
                  <button type="button" class="btn" onClick={retry}>Retry</button>
                </div>
              </Show>
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
                      <span class="block truncate">{node.preview || treeKindLabel(node.kind)}</span>
                      <span class="block truncate ttab-sub">
                        {treeKindLabel(node.kind)}
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
