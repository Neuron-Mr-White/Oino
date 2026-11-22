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
import { VirtualWindow, useVirtual } from '../../../ui/virtual';
import { buildTreeNodes, buildTreeNodesFromEntries, errorText, filterTree, flattenTree, readTreeFilter, rememberTreeFilter, TREE_FILTERS, TREE_MAX_DEPTH, treeKindLabel, type TreeNode, type TreeFilter, type TreeRow } from './tree';
import './tabs.css';

 type EntriesPage = { entries?: SessionEntry[]; leafId?: string | null; total?: number; hasMore?: boolean; cursor?: string | null };
 type Accumulated = { all: SessionEntry[]; before: string | undefined; leafId: string | null; hasMore: boolean; total: number | undefined; started: boolean };
 type WireNode = { entry: SessionEntry; children?: WireNode[]; label?: unknown; labelTimestamp?: unknown };
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
  const [scroller, setScroller] = createSignal<HTMLElement>();
  let listEl: HTMLDivElement | undefined;

  /** Rows past this mount only the ones in view (a long chat is thousands of rows). */
  const TREE_VIRTUAL_AT = 200;
  /** Fixed row height (px): `.ttab-row` is min-height 40 with two truncating lines. */
  const TREE_ROW_H = 40;
  // A request that never answers must not leave "Loading…" up forever: every step has a deadline,
  // and the last one's failure is shown with a Retry.
  const LOAD_TIMEOUT_MS = 45_000;
  const STEP_TIMEOUT_MS = 10_000;
  const TREE_HEAD_START_MS = 2_500;
  const PAGE = 500;
  /** Entries per "Load older" step, and in the first load: a 40k-entry chat never builds 40k rows at once. */
  const TREE_ENTRY_CAP = 10_000;
  /** `get_tree` (every branch, one reply) is only asked for a branch this short: a longer one cannot fit a frame. */
  const TREE_WHOLE_MAX = 1_000;
  const within = <T,>(work: Promise<T>, ms: number): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const t = setTimeout(() => reject(new Error('The host took too long to answer.')), ms);
      work.then(
        (v) => (clearTimeout(t), resolve(v)),
        (e) => (clearTimeout(t), reject(e)),
      );
    });
  let loadSeq = 0;
  /** The fallback's pages so far (kept for "Load older"); unset when `get_tree` answered whole. */
  let accumulated: Accumulated | undefined;
  /** Set while the list is only the newest `loaded` of `total` entries. */
  const [older, setOlder] = createSignal<{ total?: number; loaded: number } | undefined>();
  const [olderBusy, setOlderBusy] = createSignal(false);
  const [olderError, setOlderError] = createSignal<string | undefined>();
  /**
   * `get_tree` carries the flat `{entry, label?}` list (host [tree-flat]; the
   * nested reply it once sent recursed one serde level per entry and crashed
   * the host worker on long chats). The nested form is still accepted (older
   * hosts); the paged branch is asked first and stays the fallback.
   */
  const load = async (s: SessionHandle): Promise<{ nodes: TreeNode[]; acc?: Accumulated }> => {
    // r4 D14: the paged branch comes first. `get_tree` serialises every entry of the file on the
    // host (73 MB on a 40k-entry chat) and the host drops any reply over one frame (1 MiB), so it
    // is only asked when the first page says the whole tree is small enough to fit one.
    const acc: Accumulated = { all: [], before: undefined, leafId: null, hasMore: false, total: undefined, started: false };
    try {
      const first = await within(s.cmd<EntriesPage>({ type: 'get_entries', limit: PAGE }), STEP_TIMEOUT_MS * 2);
      if (Array.isArray(first?.entries)) take(acc, first);
    } catch {
      // the paged branch is retried below, from the start
    }
    if (!acc.started || (acc.total ?? acc.all.length) <= TREE_WHOLE_MAX) {
      try {
        const reply = await within(s.cmd<TreeReply>({ type: 'get_tree' }), TREE_HEAD_START_MS);
        return { nodes: buildTreeNodes(Array.isArray(reply?.tree) ? reply.tree : [], reply?.leafId ?? null) };
      } catch {
        // fall through to the paged branch
      }
    }
    // The branch comes in pages, newest first, up to TREE_ENTRY_CAP entries; the rest waits behind
    // "Load older" (`loadOlder`), never silently dropped.
    try {
      await pull(s, acc, TREE_ENTRY_CAP);
    } catch (e) {
      // A later page failing keeps the newest ones: "Load older" retries from there.
      if (acc.all.length === 0) throw e;
    }
    return { nodes: buildTreeNodesFromEntries(acc.all, acc.leafId), acc };
  };
  /** Folds one `get_entries` page into the accumulator (`older` = the page was asked with `before`). */
  const take = (acc: Accumulated, page: EntriesPage | undefined, more = false) => {
    const got = Array.isArray(page?.entries) ? page.entries : [];
    acc.started = true;
    if (!more) acc.leafId = page?.leafId ?? null;
    if (typeof page?.total === 'number') acc.total = page.total;
    if (got.length === 0) {
      acc.hasMore = false;
      return;
    }
    acc.all.unshift(...got);
    acc.hasMore = !!page?.hasMore;
    acc.before = page?.cursor ?? got[0]!.id;
  };
  /** Pages older entries until `target` entries are in or the branch starts. */
  const pull = async (s: SessionHandle, acc: Accumulated, target: number) => {
    while (!acc.started || (acc.hasMore && acc.all.length < target)) {
      const page = await within(
        s.cmd<EntriesPage>({ type: 'get_entries', limit: PAGE, ...(acc.before ? { before: acc.before } : {}) }),
        STEP_TIMEOUT_MS,
      );
      take(acc, page, acc.all.length > 0);
      if (!acc.hasMore) break;
    }
  };
  /** The row at the top of a windowed list and how far it is scrolled past (rows are fixed-height). */
  const readingAnchor = (): { id: string; offset: number } | undefined => {
    const el = scroller();
    if (!el || !windowed()) return undefined;
    const i = Math.min(rows().length - 1, Math.floor(el.scrollTop / TREE_ROW_H));
    const row = rows()[i];
    return row ? { id: row.node.id, offset: el.scrollTop - i * TREE_ROW_H } : undefined;
  };
  /** Puts that row back at the same place (the spacer grows a frame after the rows change, so retry a few). */
  const restoreReading = (a: { id: string; offset: number } | undefined, tries = 6) => {
    const el = scroller();
    if (!el || !a) return;
    const i = rows().findIndex((row) => row.node.id === a.id);
    if (i < 0) return;
    const want = i * TREE_ROW_H + a.offset;
    el.scrollTop = want;
    if (Math.abs(el.scrollTop - want) > 1 && tries > 0) requestAnimationFrame(() => restoreReading(a, tries - 1));
  };
  /** "Load older": the next TREE_ENTRY_CAP entries, rebuilt into the tree. */
  const loadOlder = async () => {
    const s = handle();
    const acc = accumulated;
    if (!s || !acc || !acc.hasMore || olderBusy()) return;
    const mine = loadSeq;
    setOlderBusy(true);
    setOlderError(undefined);
    try {
      await pull(s, acc, acc.all.length + TREE_ENTRY_CAP);
      if (mine !== loadSeq) return;
      // r4 D11: the older rows land above the one being read, which stays where it is.
      const keep = readingAnchor();
      setNodes(buildTreeNodesFromEntries(acc.all, acc.leafId));
      setOlder(acc.hasMore ? { total: acc.total, loaded: acc.all.length } : undefined);
      restoreReading(keep);
    } catch (e) {
      if (mine === loadSeq) setOlderError(errorText(e));
    } finally {
      if (mine === loadSeq) setOlderBusy(false);
    }
  };
  const refresh = async () => {
    const s = handle();
    if (!s) return;
    const mine = ++loadSeq;
    setOlderBusy(false);
    setOlderError(undefined);
    try {
      const next = await within(load(s), LOAD_TIMEOUT_MS);
      if (mine !== loadSeq) return;
      accumulated = next.acc;
      setOlder(next.acc?.hasMore ? { total: next.acc.total, loaded: next.acc.all.length } : undefined);
      setNodes(next.nodes);
      setError(undefined);
    } catch (e) {
      if (mine !== loadSeq) return;
      setError(errorText(e));
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
  const windowed = () => rows().length > TREE_VIRTUAL_AT;
  const virt = useVirtual({ count: () => (windowed() ? rows().length : 0), scroller, estimate: () => TREE_ROW_H, key: (i) => rows()[i]?.node.id ?? i, fixed: true, overscan: 8 });
  createEffect(() => {
    if (loading() || !listEl) return;
    queueMicrotask(() => {
      const index = rows().findIndex((row) => row.node.current);
      if (windowed() && index >= 0) virt.scrollToIndex(index, { align: 'center' });
      else listEl?.querySelector<HTMLElement>('[data-current]')?.scrollIntoView?.({ block: 'center' });
    });
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
      setError(errorText(e));
    }
  };

  const onRow = (node: TreeNode, anchor: DOMRect) => {
    // On the current branch (not the leaf) the row asks: go there, or branch.
    if (node.onPath && !node.current) return setRowMenu({ node, anchor });
    void goTo(node, summarize());
  };

  /** One tree row (shared by the plain list and the windowed one). */
  const rowView = (row: TreeRow) => {
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
      <Show when={!loading() && older()}>
        {(o) => (
          <div class="ttab-trunc" data-testid="tree-truncated" role="status">
            <span class="ttab-trunc-text">
              {o().total && o().total! > o().loaded
                ? `Showing latest ${o().loaded.toLocaleString('en-US')} of ${o().total!.toLocaleString('en-US')} entries`
                : `Showing latest ${o().loaded.toLocaleString('en-US')} entries`}
              <Show when={olderError()}> · {olderError()}</Show>
            </span>
            <button type="button" class="ttab-trunc-btn" data-testid="tree-load-older" disabled={olderBusy()} onClick={() => void loadOlder()}>
              {olderBusy() ? 'Loading…' : 'Load older'}
            </button>
          </div>
        )}
      </Show>
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
            <Show when={windowed()} fallback={<For each={rows()}>{(row) => rowView(row)}</For>}>
              {/* A long chat is thousands of rows: its own scroll box mounts
                  only the rows in view (§13.4, as the Running tab). */}
              <div class="ttab-list-scroll" ref={setScroller}>
                <VirtualWindow virt={virt} fixed>{(i) => rowView(rows()[i]!)}</VirtualWindow>
              </div>
            </Show>
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
