/**
 * One project section of a host group (Z-9/D-9, Z-11, WP-A4): the folder name
 * (full cwd in the tooltip), sessions ordered pinned → running → needs-you →
 * recent, 5 rows with a "Show more" that adds 5 (ZCode `workspace-sidebar-
 * navigation.md`: no "Show less"; the paging resets when the host group
 * collapses). A project with more than 50 sessions renders its rows
 * windowed (the acceptance's virtualization), so a huge project cannot
 * flood the DOM.
 */
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Archive, Plus } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import type { SessionMeta } from '../../lib/session/hub';
import { NO_WORKSPACE_LABEL, shortCwd } from '../../lib/session/selectors';
import { SessionRow, type ModeBadge, type RowProps, type RowVariant } from './SessionRow';

/** Rows shown before "Show more" (Z-11). */
export const ROWS_PER_PROJECT = 5;
/** A project bigger than this renders its rows windowed (WP-A4 acceptance). */
export const VIRTUALIZE_AT = 50;
/** Rows rendered around the window (jsdom's zero-height viewports still show some). */
const WINDOW_OVERSCAN = 8;
const WINDOW_MIN = 12;

type Paging = { get: (key: string) => number; more: (key: string, step?: number) => void; reset: () => void };

/** Row slot heights (px) for the windowed list: comfortable / compact × sidebar / page. */
export function rowSlot(variant: RowVariant, compact: boolean): number {
  if (variant === 'page') return compact ? 44 : 60;
  return compact ? 30 : 44;
}

export function ProjectGroup(props: {
  hostId: string;
  /** The project folder (`""` = No workspace) — absent for non-project groupings (no `+` draft). */
  cwd?: string;
  /** Paging key (defaults to the cwd). */
  groupKey?: string;
  label: string;
  cards: SessionCard[];
  meta: Readonly<Record<string, SessionMeta>>;
  variant: RowVariant;
  paging: Paging;
  modeOf: (card: SessionCard) => ModeBadge | undefined;
  onPin: (card: SessionCard, pinned: boolean) => void;
  activeOf: (card: SessionCard) => boolean;
  /** Properties › visibility / compact rows. */
  show?: RowProps;
  compact?: boolean;
  /** Archived chats (the Archived filter shows the trash inline). */
  archivedOf?: (card: SessionCard) => boolean;
  /** No head row (Group by: None — one flat list). */
  headless?: boolean;
  /** Rows per "Show more" (default 5, Z-11). */
  page?: number;
  /** "Archive all in <group>" on the head (asks first, Undo toast). */
  onArchiveAll?: (anchor: DOMRect) => void;
  /** The subagent tree under a row (opt-in): its element and extra height in px. */
  treeOf?: (card: SessionCard) => { height: number; render: () => JSX.Element } | undefined;
}): JSX.Element {
  const key = () => props.groupKey ?? props.cwd ?? props.label;
  const step = () => props.page ?? ROWS_PER_PROJECT;
  const visible = createMemo(() => {
    const limit = Math.max(props.paging.get(key()), step());
    return limit >= props.cards.length ? props.cards : props.cards.slice(0, limit);
  });
  const hidden = () => props.cards.length - visible().length;
  const rowHeight = () => rowSlot(props.variant, !!props.compact);
  const isProject = () => props.cwd !== undefined;

  // >50 sessions: a positioned window over one shared scroll parent. Rows with an open
  // subagent tree are taller, so the window walks prefix offsets, not i × height.
  let container: HTMLDivElement | undefined;
  const virtual = () => props.cards.length > VIRTUALIZE_AT;
  const layoutRows = createMemo(() => {
    const tops: number[] = [];
    const trees: (ReturnType<NonNullable<typeof props.treeOf>>)[] = [];
    let y = 0;
    for (const card of visible()) {
      tops.push(y);
      const tree = props.treeOf?.(card);
      trees.push(tree);
      y += rowHeight() + (tree?.height ?? 0);
    }
    return { tops, trees, height: y };
  });
  const [window_, setWindow] = createSignal({ from: 0, to: WINDOW_MIN * 44 });
  const measure = () => {
    if (!virtual() || !container?.isConnected) return;
    const scroller = scrollParent(container);
    const top = container.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
    const height = scroller.clientHeight || window.innerHeight || 640;
    const pad = WINDOW_OVERSCAN * rowHeight();
    const from = Math.max(0, scroller.scrollTop - top - pad);
    const to = Math.max(scroller.scrollTop - top + height + pad, WINDOW_MIN * rowHeight());
    setWindow((prev) => (prev.from === from && prev.to === to ? prev : { from, to }));
  };
  onMount(() => {
    const el = container;
    if (!el) return;
    const scroller = scrollParent(el);
    scroller.addEventListener('scroll', measure, { passive: true });
    window.addEventListener('resize', measure);
    measure();
    onCleanup(() => {
      scroller.removeEventListener('scroll', measure);
      window.removeEventListener('resize', measure);
    });
  });

  const rows = createMemo(() => {
    const { tops, trees } = layoutRows();
    const all = visible().map((card, i) => ({ card, i, tree: trees[i] }));
    if (!virtual()) return all.map((r) => ({ ...r, top: undefined as number | undefined }));
    const { from, to } = window_();
    return all.filter(({ i }) => tops[i]! + rowHeight() + (trees[i]?.height ?? 0) >= from && tops[i]! <= to).map((r) => ({ ...r, top: tops[r.i] }));
  });

  return (
    <div class="sb-project" data-testid={`sb-project-${props.label}`} data-headless={props.headless ? '' : undefined}>
      <Show when={!props.headless}>
        <div class="sb-project-head" title={isProject() ? (props.cwd ? shortCwd(props.cwd) : NO_WORKSPACE_LABEL) : props.label} data-testid="sb-project-head">
          <span class="sb-project-label">{props.label || NO_WORKSPACE_LABEL}</span>
          <Show when={props.cards.length > 1}>
            <span class="sb-project-count" title={`${props.cards.length} sessions`}>{props.cards.length}</span>
          </Show>
          <span class="sb-project-tools">
            <Show when={props.onArchiveAll}>
              <button
                type="button"
                class="sb-project-tool"
                title={`Archive all in ${props.label || NO_WORKSPACE_LABEL}`}
                aria-label={`Archive all in ${props.label || NO_WORKSPACE_LABEL}`}
                data-testid={`sb-project-archive-${props.label}`}
                onClick={(e) => {
                  e.stopPropagation();
                  props.onArchiveAll!((e.currentTarget as HTMLElement).getBoundingClientRect());
                }}
              >
                <Archive size={13} />
              </button>
            </Show>
            {/* Z-19 (WP-A5): a per-project draft in this folder (`/new/:host?cwd=`); a
                project-less group starts the host's default draft ("No workspace", D-18). */}
            <Show when={isProject()}>
              <a
                class="sb-project-new"
                href={`#/new/${encodeURIComponent(props.hostId)}${props.cwd ? `?cwd=${encodeURIComponent(props.cwd)}` : ''}`}
                title={props.cwd ? 'New chat in this folder' : 'New chat'}
                aria-label={props.cwd ? `New chat in ${props.label || NO_WORKSPACE_LABEL}` : 'New chat'}
                data-testid={`sb-project-new-${props.label}`}
                onClick={(e) => e.stopPropagation()}
              >
                <Plus size={13} />
              </a>
            </Show>
          </span>
        </div>
      </Show>
      <div
        ref={container}
        class="sb-rows"
        classList={{ 'sb-rows-virtual': virtual() }}
        style={virtual() ? { height: `${layoutRows().height}px`, position: 'relative' } : undefined}
        data-testid={`sb-rows-${props.label}`}
      >
        <For each={rows()}>
          {(row) => (
            <div class="sb-row-slot" style={row.top !== undefined ? { position: 'absolute', top: `${row.top}px`, left: 0, right: 0 } : undefined}>
              <SessionRow
                hostId={props.hostId}
                card={row.card}
                meta={props.meta[row.card.session_id]}
                active={props.activeOf(row.card)}
                variant={props.variant}
                mode={props.modeOf(row.card)}
                show={props.show}
                compact={props.compact}
                archived={props.archivedOf?.(row.card)}
                onPin={props.onPin}
              />
              <Show when={row.tree}>{(t) => t().render()}</Show>
            </div>
          )}
        </For>
      </div>
      <Show when={hidden() > 0}>
        <button
          type="button"
          class="sb-more"
          data-testid={`sb-more-${props.label}`}
          title={`Show ${Math.min(hidden(), step())} more of ${hidden()} hidden`}
          onClick={() => props.paging.more(key(), step())}
        >
          Show more
          <span class="sb-more-count">{Math.min(hidden(), step())}</span>
        </button>
      </Show>
    </div>
  );
}

/** The nearest scrolling ancestor (the sidebar scroll area / the page). */
function scrollParent(el: HTMLElement): HTMLElement {
  let node: HTMLElement | null = el.parentElement;
  while (node) {
    const overflow = window.getComputedStyle(node).overflowY;
    if (overflow === 'auto' || overflow === 'scroll' || node === document.documentElement) return node;
    node = node.parentElement;
  }
  return document.documentElement;
}

/** Paging state for one host group: per-project limits, reset together (Z-11). */
export function createPaging(initial = ROWS_PER_PROJECT): Paging {
  const [limits, setLimits] = createSignal<Record<string, number>>({});
  return {
    get: (key) => limits()[key] ?? initial,
    more: (key, step = ROWS_PER_PROJECT) => setLimits((cur) => ({ ...cur, [key]: Math.max(cur[key] ?? initial, step) + step })),
    reset: () => setLimits({}),
  };
}
