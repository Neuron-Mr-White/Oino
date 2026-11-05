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
import { Check } from 'lucide-solid';
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
  const leftFor = (actualW = width()) => {
    const vw = window.innerWidth;
    const w = Math.min(actualW, Math.max(120, vw - GAP * 2));
    const raw = props.alignEnd ? props.anchor.right - w : props.anchor.left;
    return Math.max(GAP, Math.min(raw, vw - w - GAP));
  };
  const [pos, setPos] = createSignal({
    left: leftFor(),
    top: props.anchor.bottom + GAP,
    maxHeight: Math.max(160, (typeof window !== 'undefined' ? window.innerHeight : 800) - GAP * 2),
    origin: 'top left',
  });
  const reposition = () => {
    if (!menu) return;
    // offsetHeight / scrollHeight, not getBoundingClientRect: the popover-in animation scales
    // the menu (0.94) on mount, and dynamic sub-panels (Filter / Properties / async items) can
    // grow taller after open — re-clamping on every size/DOM change keeps the longest state clickable.
    const vh = window.innerHeight;
    const vw = window.innerWidth;
    const nav = document.querySelector('.bottom-nav')?.getBoundingClientRect().height ?? 0;
    const floor = vh - nav - GAP;
    const maxAvail = Math.max(120, floor - GAP);
    const natural = Math.max(menu.offsetHeight || 0, menu.scrollHeight || 0);
    const height = Math.min(natural, maxAvail);
    let top = props.anchor.bottom + GAP;
    let origin = 'top';
    if (top + height > floor) {
      const above = props.anchor.top - GAP - height;
      top = above >= GAP ? above : Math.max(GAP, floor - height);
      origin = above >= GAP ? 'bottom' : 'center';
    }
    const maxHeight = Math.max(120, floor - top);
    const actualW = Math.min(Math.max(width(), menu.offsetWidth || 0), Math.max(120, vw - GAP * 2));
    setPos({ left: leftFor(actualW), top, maxHeight, origin: `${origin} ${props.alignEnd ? 'right' : 'left'}` });
  };
  onMount(() => {
    reposition();
    if (typeof ResizeObserver !== 'undefined' && menu) {
      const ro = new ResizeObserver(() => reposition());
      ro.observe(menu);
      onCleanup(() => ro.disconnect());
    }
    if (typeof MutationObserver !== 'undefined' && menu) {
      const mo = new MutationObserver(() => reposition());
      mo.observe(menu, { childList: true, subtree: true });
      onCleanup(() => mo.disconnect());
    }
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
        style={{
          left: `${pos().left}px`,
          top: `${pos().top}px`,
          width: `min(${width()}px, calc(100vw - 16px))`,
          'max-height': `${pos().maxHeight}px`,
          'transform-origin': pos().origin,
        }}
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

/**
 * A checkbox row (the chat filter): the tick shows the state, the row toggles
 * it. The menu's arrow keys walk it like any other row.
 */
export function MenuCheckRow(props: { label: string; checked: boolean; testId?: string; onSelect: () => void }) {
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={props.checked}
      class="recent-popover-row board-menu-row board-menu-check"
      data-testid={props.testId}
      data-checked={props.checked ? '' : undefined}
      onClick={() => props.onSelect()}
    >
      <span class="board-menu-icon">
        <Check size={16} />
      </span>
      <span class="min-w-0 flex-1 truncate text-left">{props.label}</span>
    </button>
  );
}

export function MenuLabel(props: { children: JSX.Element }) {
  return <div class="board-menu-label">{props.children}</div>;
}
