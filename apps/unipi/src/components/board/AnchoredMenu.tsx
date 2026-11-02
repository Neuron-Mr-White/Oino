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
import { layout } from '../../lib/layout';
import { SheetFrame } from '../phone/Sheet';
import './anchored-menu.css';

/** The menu's enabled items, in order (roving focus for arrow keys). */
function menuItems(menu: HTMLElement | undefined): HTMLElement[] {
  if (!menu) return [];
  return Array.from(menu.querySelectorAll<HTMLElement>('[role=menuitem],[role=menuitemradio],[role=menuitemcheckbox]')).filter(
    (el) => !(el as HTMLButtonElement).disabled && el.getAttribute('aria-disabled') !== 'true',
  );
}

type AnchoredMenuProps = {
  anchor: DOMRect;
  onDismiss: () => void;
  label: string;
  width?: number;
  class?: string;
  /** Align the menu's right edge with the anchor's (header ⋯ buttons). */
  alignEnd?: boolean;
  /** Phone (action sheet) title; the accessible label is used when absent. */
  title?: string;
  children: JSX.Element;
};

/**
 * Phone spec §2.12: on the phone shell every anchored menu is an action sheet
 * around the same rows (MenuRow → 52 px row, MenuSeparator → section gap), so
 * all ~30 menus convert at once; the desktop keeps the popover byte-for-byte.
 */
export function AnchoredMenu(props: AnchoredMenuProps) {
  return (
    <Show when={phoneShell()} fallback={<AnchoredPopover {...props} />}>
      <MenuSheet {...props} />
    </Show>
  );
}

/** `html[data-shell=phone]` (set by the app shell): bare component tests keep the popover. */
function phoneShell(): boolean {
  return !layout().desktop() && typeof document !== 'undefined' && document.documentElement.dataset.shell === 'phone';
}

function MenuSheet(props: AnchoredMenuProps) {
  return (
    <SheetFrame open onClose={() => props.onDismiss()} label={props.label} title={props.title ?? false} class="ph-menu-sheet" testId="anchored-menu-sheet">
      <div role="menu" aria-label={props.label} class={`recent-popover board-menu${props.class ? ` ${props.class}` : ''}`} onClick={(event) => event.stopPropagation()}>
        {props.children}
      </div>
    </SheetFrame>
  );
}

function AnchoredPopover(props: AnchoredMenuProps) {
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
    // Keyboard: the first item takes focus (arrows / Home / End move it,
    // Escape closes via the Back stack); focus goes back where it was.
    const opener = document.activeElement as HTMLElement | null;
    if (!menu?.contains(document.activeElement)) menuItems(menu)[0]?.focus({ preventScroll: true });
    onCleanup(() => {
      const active = document.activeElement;
      if (opener?.isConnected && (!active || active === document.body || menu?.contains(active))) opener.focus({ preventScroll: true });
    });
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
        class={`recent-popover board-menu${props.class ? ` ${props.class}` : ''}`}
        data-leaving={leave.leaving() ? '' : undefined}
        style={{ left: `${pos().left}px`, top: `${pos().top}px`, width: `${width()}px`, 'transform-origin': pos().origin }}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          // A menu wired with `menuKeys` already moved the focus (or closed) for this key.
          if (event.defaultPrevented) return;
          const items = menuItems(menu);
          if (items.length === 0) return;
          const at = items.indexOf(document.activeElement as HTMLElement);
          let next: number | undefined;
          if (event.key === 'ArrowDown') next = at < 0 ? 0 : (at + 1) % items.length;
          else if (event.key === 'ArrowUp') next = at < 0 ? items.length - 1 : (at - 1 + items.length) % items.length;
          else if (event.key === 'Home') next = 0;
          else if (event.key === 'End') next = items.length - 1;
          else if (event.key === 'Tab') {
            event.preventDefault();
            dismiss();
            return;
          }
          if (next === undefined) return;
          event.preventDefault();
          event.stopPropagation();
          items[next]!.focus({ preventScroll: true });
        }}
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
  testId?: string;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      class={`recent-popover-row board-menu-row${props.tone === 'danger' ? ' text-wm-red' : ''}`}
      data-testid={props.testId}
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
