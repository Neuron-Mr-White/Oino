/**
 * Floating anchored menu (the Sessions long-press "recent-popover"
 * pattern, shared for the Board): portaled, `position: fixed`, placed
 * under (or above) the anchor rect and clamped to the viewport above the
 * bottom nav, with a transparent tap-to-dismiss backdrop. Android Back
 * closes it. UNI-131 uses it for the card long-press quick menu.
 */
import { createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { pushBack } from '../../lib/back';
import { createExit } from '../../lib/motion';

export function AnchoredMenu(props: {
  anchor: DOMRect;
  onDismiss: () => void;
  label: string;
  width?: number;
  /** Align the menu's right edge with the anchor's (header ⋯ buttons). */
  alignEnd?: boolean;
  children: JSX.Element;
}) {
  let menu: HTMLDivElement | undefined;
  // Z-91: an outside click / Escape / resize fades the menu out before it leaves.
  const leave = createExit(() => props.onDismiss());
  const dismiss = () => leave.begin();
  const width = () => props.width ?? 240;
  const GAP = 8;
  const leftFor = () => {
    const vw = window.innerWidth;
    const raw = props.alignEnd ? props.anchor.right - width() : props.anchor.left;
    return Math.max(GAP, Math.min(raw, vw - width() - GAP));
  };
  const [pos, setPos] = createSignal({ left: leftFor(), top: props.anchor.bottom + GAP, origin: 'top left' });
  onMount(() => {
    // offsetHeight, not getBoundingClientRect: the popover-in animation is scaling the menu
    // (0.94) right now, which under-measured it and let tall menus run off the bottom edge.
    const height = menu?.offsetHeight ?? 0;
    const vh = window.innerHeight;
    const nav = document.querySelector('.bottom-nav')?.getBoundingClientRect().height ?? 0;
    const floor = vh - nav - GAP;
    let top = props.anchor.bottom + GAP;
    let origin = 'top';
    if (top + height > floor) {
      const above = props.anchor.top - GAP - height;
      top = above >= GAP ? above : Math.max(GAP, floor - height);
      origin = above >= GAP ? 'bottom' : 'center';
    }
    setPos({ left: leftFor(), top, origin: `${origin} ${props.alignEnd ? 'right' : 'left'}` });
    onCleanup(pushBack(() => dismiss()));
    window.addEventListener('resize', dismiss);
    onCleanup(() => window.removeEventListener('resize', dismiss));
  });
  return (
    <Portal>
      <div
        class="recent-popover-backdrop"
        data-leaving={leave.leaving() ? '' : undefined}
        data-testid="anchored-menu-backdrop"
        onClick={(event) => {
          event.stopPropagation();
          dismiss();
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          dismiss();
        }}
        onTouchMove={() => dismiss()}
      />
      <div
        ref={menu}
        role="menu"
        aria-label={props.label}
        class="recent-popover board-menu"
        data-leaving={leave.leaving() ? '' : undefined}
        style={{ left: `${pos().left}px`, top: `${pos().top}px`, width: `${width()}px`, 'transform-origin': pos().origin }}
        onClick={(event) => event.stopPropagation()}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
        }}
      >
        {props.children}
      </div>
    </Portal>
  );
}

export function MenuRow(props: {
  icon?: JSX.Element;
  label: string;
  hint?: JSX.Element;
  tone?: 'danger';
  disabled?: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      class={`recent-popover-row board-menu-row${props.tone === 'danger' ? ' text-wm-red' : ''}`}
      disabled={props.disabled}
      onClick={() => props.onSelect()}
    >
      <Show when={props.icon}>
        <span class="board-menu-icon">{props.icon}</span>
      </Show>
      <span class="min-w-0 flex-1 truncate text-left">{props.label}</span>
      <Show when={props.hint}>
        <span class="board-menu-hint">{props.hint}</span>
      </Show>
    </button>
  );
}

export function MenuSeparator() {
  return <div class="board-menu-sep" role="separator" />;
}

export function MenuLabel(props: { children: JSX.Element }) {
  return <div class="board-menu-label">{props.children}</div>;
}
