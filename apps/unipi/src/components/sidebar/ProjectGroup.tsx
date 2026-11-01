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
import { Plus } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import type { SessionMeta } from '../../lib/session/hub';
import { NO_WORKSPACE_LABEL, shortCwd } from '../../lib/session/selectors';
import { SessionRow, type ModeBadge, type RowVariant } from './SessionRow';

/** Rows shown before "Show more" (Z-11). */
export const ROWS_PER_PROJECT = 5;
/** A project bigger than this renders its rows windowed (WP-A4 acceptance). */
export const VIRTUALIZE_AT = 50;
/** Rows rendered around the window (jsdom's zero-height viewports still show some). */
const WINDOW_OVERSCAN = 8;
const WINDOW_MIN = 12;

type Paging = { get: (key: string) => number; more: (key: string) => void; reset: () => void };

export function ProjectGroup(props: {
  hostId: string;
  cwd: string;
  label: string;
  cards: SessionCard[];
  meta: Readonly<Record<string, SessionMeta>>;
  variant: RowVariant;
  paging: Paging;
  modeOf: (card: SessionCard) => ModeBadge | undefined;
  onPin: (card: SessionCard, pinned: boolean) => void;
  activeOf: (card: SessionCard) => boolean;
}): JSX.Element {
  const visible = createMemo(() => {
    const limit = props.paging.get(props.cwd);
    return limit >= props.cards.length ? props.cards : props.cards.slice(0, limit);
  });
  const hidden = () => props.cards.length - visible().length;
  const rowHeight = () => (props.variant === 'page' ? 60 : 36);

  // >50 sessions: a positioned window over one shared scroll parent.
  let container: HTMLDivElement | undefined;
  const virtual = () => props.cards.length > VIRTUALIZE_AT;
  const [window_, setWindow] = createSignal({ start: 0, count: WINDOW_MIN });
  const measure = () => {
    if (!virtual() || !container?.isConnected) return;
    const scroller = scrollParent(container);
    const top = container.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
    const height = scroller.clientHeight || window.innerHeight || 640;
    const first = Math.max(0, Math.floor((scroller.scrollTop - top) / rowHeight()) - WINDOW_OVERSCAN);
    const count = Math.ceil(height / rowHeight()) + WINDOW_OVERSCAN * 2;
    setWindow((prev) => (prev.start === first && prev.count === count ? prev : { start: first, count: Math.max(count, WINDOW_MIN) }));
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
    if (!virtual()) return visible().map((card, i) => ({ card, top: undefined as number | undefined, i }));
    const { start, count } = window_();
    return visible()
      .map((card, i) => ({ card, i }))
      .filter(({ i }) => i >= start && i < start + count)
      .map(({ card, i }) => ({ card, top: i * rowHeight(), i }));
  });

  return (
    <div class="sb-project" data-testid={`sb-project-${props.label}`}>
      <div class="sb-project-head" title={props.cwd ? shortCwd(props.cwd) : NO_WORKSPACE_LABEL} data-testid="sb-project-head">
        <span class="sb-project-label">{props.label || NO_WORKSPACE_LABEL}</span>
        <Show when={props.cards.length > 1}>
          <span class="sb-project-count" title={`${props.cards.length} sessions`}>{props.cards.length}</span>
        </Show>
        {/* Z-19 (WP-A5): a per-project draft in this folder (`/new/:host?cwd=`); a
            project-less group starts the host's default draft ("No workspace", D-18). */}
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
      </div>
      <div
        ref={container}
        class="sb-rows"
        classList={{ 'sb-rows-virtual': virtual() }}
        style={virtual() ? { height: `${visible().length * rowHeight()}px`, position: 'relative' } : undefined}
        data-testid={`sb-rows-${props.label}`}
      >
        <For each={rows()}>
          {({ card, top }) => (
            <div style={top !== undefined ? { position: 'absolute', top: `${top}px`, left: 0, right: 0 } : undefined}>
              <SessionRow
                hostId={props.hostId}
                card={card}
                meta={props.meta[card.session_id]}
                active={props.activeOf(card)}
                variant={props.variant}
                mode={props.modeOf(card)}
                onPin={props.onPin}
              />
            </div>
          )}
        </For>
      </div>
      <Show when={hidden() > 0}>
        <button
          type="button"
          class="sb-more"
          data-testid={`sb-more-${props.label}`}
          title={`Show ${Math.min(hidden(), ROWS_PER_PROJECT)} more of ${hidden()} hidden`}
          onClick={() => props.paging.more(props.cwd)}
        >
          Show more
          <span class="sb-more-count">{Math.min(hidden(), ROWS_PER_PROJECT)}</span>
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
    more: (key) => setLimits((cur) => ({ ...cur, [key]: (cur[key] ?? initial) + ROWS_PER_PROJECT })),
    reset: () => setLimits({}),
  };
}
