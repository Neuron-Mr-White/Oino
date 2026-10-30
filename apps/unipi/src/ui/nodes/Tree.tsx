/**
 * `Tree` (ui-protocol §4.5.2): `role="tree"` rows (32 / 44 px) flattened to
 * the visible set, virtualized above 100 visible rows. Expansion and
 * selection are local; a node's `expanded` (and `state.selected`) wins when
 * it changes. ↑↓ move, → expands / enters, ← collapses / goes to the parent,
 * Enter activates. `onToggle` → `{key, expanded}` (lazy nodes: the extension
 * replies with children via a patch).
 */
import { For, Show, createComputed, createMemo, createSignal, type JSX } from 'solid-js';
import { ChevronRight } from 'lucide-solid';
import { useEntry } from '../actions';
import { useLocal } from '../localState';
import { shell } from '../layout/units';
import type { TreeNode, TreeViewNode } from '../types';
import { VIRTUAL_THRESHOLD, VirtualWindow, useVirtual } from '../virtual';
import { IconView } from './icons';
import { InlineLine } from './Text';
import { navIndex } from './List';
import { baseStyle, cx, type NodeProps } from './props';

export type FlatRow = { node: TreeNode; depth: number; parent?: string; expandable: boolean; expanded: boolean; index: number; setSize: number };

/** Flattens the visible rows (children of collapsed nodes are skipped). */
export function flattenTree(nodes: readonly TreeNode[], isOpen: (n: TreeNode) => boolean): FlatRow[] {
  const out: FlatRow[] = [];
  const walk = (list: readonly TreeNode[], depth: number, parent?: string) => {
    list.forEach((n, index) => {
      const expandable = !!n.lazy || (Array.isArray(n.children) && n.children.length > 0);
      const expanded = expandable && isOpen(n);
      out.push({ node: n, depth, parent, expandable, expanded, index, setSize: list.length });
      if (expanded && n.children) walk(n.children, depth + 1, n.key);
    });
  };
  walk(Array.isArray(nodes) ? nodes : [], 0);
  return out;
}

function propMap(nodes: readonly TreeNode[] | undefined, into: Record<string, boolean | undefined> = {}): Record<string, boolean | undefined> {
  for (const n of nodes ?? []) {
    into[n.key] = n.expanded;
    propMap(n.children, into);
  }
  return into;
}

export function Tree(props: NodeProps<TreeViewNode>) {
  const entry = useEntry();
  const [overrides, setOverrides] = useLocal<Record<string, boolean>>(entry.entryId, props.path, 'expanded', { init: () => ({}) });
  // "Prop changed ⇒ adopt" per node: a changed `expanded` drops the local override.
  let prev: Record<string, boolean | undefined> | undefined;
  createComputed(() => {
    const next = propMap(props.node.nodes);
    if (prev) {
      const o = { ...overrides() };
      let changed = false;
      for (const k of Object.keys(next)) if (next[k] !== prev[k] && k in o) (delete o[k], (changed = true));
      if (changed) setOverrides(o);
    }
    prev = next;
  });
  const isOpen = (n: TreeNode) => overrides()[n.key] ?? !!n.expanded;
  const rows = createMemo(() => flattenTree(props.node.nodes ?? [], isOpen));
  const [selected, setSelected] = useLocal<string | undefined>(entry.entryId, props.path, 'selected', {
    prop: () => props.node.state?.selected,
    init: () => undefined,
  });
  const [scrollEl, setScrollEl] = createSignal<HTMLElement>();
  const virtual = () => rows().length > VIRTUAL_THRESHOLD;
  const virt = useVirtual({
    count: () => (virtual() ? rows().length : 0),
    scroller: scrollEl,
    estimate: () => (shell() === 'desktop' ? 32 : 44),
    key: (i) => rows()[i]?.node.key ?? i,
    fixed: true,
  });
  const idOf = (k: string) => `${entry.entryId}:${props.path.join('/')}:${k}`;

  const toggle = (r: FlatRow) => {
    if (!r.expandable) return;
    const next = !r.expanded;
    setOverrides({ ...overrides(), [r.node.key]: next });
    entry.send(props.node.onToggle, { key: r.node.key, expanded: next }, props.path);
  };
  const select = (k: string) => {
    if (selected() === k) return;
    setSelected(k);
    entry.send(props.node.onSelect, k, props.path);
  };
  const activate = (k: string) => entry.send(props.node.onActivate, k, props.path);
  const reveal = (i: number) => {
    if (virtual()) virt.scrollToIndex(i, { align: 'auto' });
    else scrollEl()?.querySelector<HTMLElement>(`[data-index="${i}"]`)?.scrollIntoView?.({ block: 'nearest' });
  };

  const onKey = (e: KeyboardEvent) => {
    const list = rows();
    const cur = list.findIndex((r) => r.node.key === selected());
    const r = list[cur];
    const nav = navIndex(e.key, cur, list.length);
    if (nav !== undefined) {
      e.preventDefault();
      select(list[nav]!.node.key);
      reveal(nav);
      return;
    }
    if (!r) return;
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      if (r.expandable && !r.expanded) toggle(r);
      else if (r.expanded && list[cur + 1]?.parent === r.node.key) select(list[cur + 1]!.node.key);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      if (r.expanded) toggle(r);
      else if (r.parent) select(r.parent);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      activate(r.node.key);
    }
  };

  const Row = (p: { index: number }): JSX.Element => {
    const r = () => rows()[p.index]!;
    const sel = () => r().node.key === selected();
    return (
      <div
        role="treeitem"
        id={idOf(r().node.key)}
        class="ui-row ui-tree-row"
        data-index={p.index}
        data-selected={sel() ? '' : undefined}
        aria-selected={sel()}
        aria-level={r().depth + 1}
        aria-posinset={r().index + 1}
        aria-setsize={r().setSize}
        aria-expanded={r().expandable ? r().expanded : undefined}
        style={{ '--ui-depth': r().depth } as JSX.CSSProperties}
        onClick={() => {
          select(r().node.key);
          if (shell() === 'phone' && !r().expandable) activate(r().node.key);
        }}
        onDblClick={() => (r().expandable ? toggle(r()) : activate(r().node.key))}
      >
        <span class="ui-tree-indent" aria-hidden="true" />
        <Show when={r().expandable} fallback={<span class="ui-tree-twisty" aria-hidden="true" />}>
          <button
            type="button"
            class="ui-tree-twisty"
            tabindex="-1"
            aria-label={r().expanded ? 'Collapse' : 'Expand'}
            data-open={r().expanded ? '' : undefined}
            onClick={(e) => {
              e.stopPropagation();
              toggle(r());
            }}
          >
            <ChevronRight size={14} aria-hidden="true" />
          </button>
        </Show>
        <Show when={r().node.icon}>
          <IconView icon={r().node.icon} size={16} class="ui-row-icon" />
        </Show>
        <span class="ui-row-main">
          <InlineLine line={r().node.label} class="ui-row-content" />
        </span>
        <Show when={r().node.badge}>
          <InlineLine line={r().node.badge} class="ui-row-badge" />
        </Show>
      </div>
    );
  };

  return (
    <div
      ref={setScrollEl}
      class={cx('ui-list ui-tree', baseStyle(props.node).class)}
      role="tree"
      tabindex="0"
      aria-activedescendant={selected() ? idOf(selected()!) : undefined}
      data-mode={props.mode}
      data-virtual={virtual() ? '' : undefined}
      style={baseStyle(props.node).style}
      onKeyDown={onKey}
    >
      <Show when={virtual()} fallback={<For each={rows()}>{(_, i) => <Row index={i()} />}</For>}>
        <VirtualWindow virt={virt} fixed>
          {(i) => <Row index={i} />}
        </VirtualWindow>
      </Show>
    </div>
  );
}
