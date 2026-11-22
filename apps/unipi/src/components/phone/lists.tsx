/**
 * Phone lists (phone spec §2.5): ListGroup (inset card or full-bleed),
 * ListRow (56 / 72 / 88 px, long-press 450 ms + impact haptic, one swipe
 * action per side), RowIcon (24 icon in a 40 tinted rounded square).
 */
import { createSignal, onCleanup, Show, type Component, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { ChevronRight } from 'lucide-solid';
import { haptic } from './services';

type Icon = Component<{ size?: number; strokeWidth?: number; class?: string }>;

export const LONG_PRESS_MS = 450;
/** Fraction of the row width a swipe must cross to trigger its action. */
export const SWIPE_TRIGGER = 0.35;

export function ListGroup(p: { title?: string; footer?: JSX.Element; inset?: boolean; children: JSX.Element }) {
  return (
    <section class="ph-group" data-inset={p.inset ? '' : undefined}>
      <Show when={p.title}>
        <h2 class="ph-group-title">{p.title}</h2>
      </Show>
      <div class="ph-group-body" role="list">
        {p.children}
      </div>
      <Show when={p.footer}>
        <div class="ph-group-foot">{p.footer}</div>
      </Show>
    </section>
  );
}

export type SwipeAction = { label: string; icon: Icon; tone: 'neutral' | 'accent' | 'danger'; onTrigger: () => void };

/** Pure: has a horizontal drag of `dx` on a `width` row crossed the trigger line? */
export function swipeTriggered(dx: number, width: number): boolean {
  return width > 0 && Math.abs(dx) >= width * SWIPE_TRIGGER;
}

export function ListRow(p: {
  title: JSX.Element;
  subtitle?: JSX.Element;
  caption?: JSX.Element;
  leading?: JSX.Element;
  trailing?: JSX.Element;
  chevron?: boolean;
  href?: string;
  onClick?: () => void;
  onLongPress?: () => void;
  selected?: boolean;
  disabled?: boolean;
  dim?: boolean;
  wrap?: boolean;
  tone?: 'danger';
  swipeStart?: SwipeAction;
  swipeEnd?: SwipeAction;
  /** OIN-37: a themed strip on the row's left edge (a Lead session); the text is its accessible label. */
  strip?: string;
  testId?: string;
}) {
  const [dx, setDx] = createSignal(0);
  const [dragging, setDragging] = createSignal(false);
  let wrap: HTMLDivElement | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let start: { x: number; y: number; id: number } | undefined;
  let mode: 'idle' | 'press' | 'swipe' | 'scroll' = 'idle';
  let longFired = false;
  let crossed = false;

  const clear = () => {
    if (timer) clearTimeout(timer);
    timer = undefined;
  };
  onCleanup(clear);

  const fireLong = () => {
    if (!p.onLongPress || p.disabled) return;
    longFired = true;
    haptic('impact');
    p.onLongPress();
  };

  const onPointerDown = (e: PointerEvent) => {
    if (p.disabled || (e.pointerType === 'mouse' && e.button !== 0)) return;
    start = { x: e.clientX, y: e.clientY, id: e.pointerId };
    mode = 'press';
    longFired = false;
    crossed = false;
    clear();
    if (p.onLongPress) timer = setTimeout(fireLong, LONG_PRESS_MS);
  };
  const onPointerMove = (e: PointerEvent) => {
    if (!start || e.pointerId !== start.id) return;
    const mx = e.clientX - start.x;
    const my = e.clientY - start.y;
    if (mode === 'press' && (Math.abs(mx) > 8 || Math.abs(my) > 8)) {
      clear();
      const canSwipe = e.pointerType !== 'mouse' && (mx > 0 ? !!p.swipeStart : !!p.swipeEnd);
      mode = canSwipe && Math.abs(mx) > Math.abs(my) * 1.4 ? 'swipe' : 'scroll';
      if (mode === 'swipe') {
        setDragging(true);
        (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
      }
    }
    if (mode === 'swipe') {
      const allowed = mx > 0 ? (p.swipeStart ? mx : 0) : p.swipeEnd ? mx : 0;
      setDx(allowed);
      const now = swipeTriggered(allowed, wrap?.clientWidth ?? 0);
      if (now !== crossed) {
        crossed = now;
        if (now) haptic('impact');
      }
    }
  };
  const finish = () => {
    clear();
    if (mode === 'swipe') {
      const width = wrap?.clientWidth ?? 0;
      const value = dx();
      const action = value > 0 ? p.swipeStart : p.swipeEnd;
      setDragging(false);
      setDx(0);
      if (action && swipeTriggered(value, width)) action.onTrigger();
      // Swallow the click that follows the drag.
      longFired = true;
    }
    start = undefined;
    mode = 'idle';
  };

  const content = () => (
    <>
      <Show when={p.strip}>
        {(label) => <span class="ph-row-strip" role="img" aria-label={label()} title={label()} />}
      </Show>
      <Show when={p.leading}>
        <span class="ph-row-lead">{p.leading}</span>
      </Show>
      <span class="ph-row-text">
        <span class="ph-row-title">{p.title}</span>
        <Show when={p.subtitle}>
          <span class="ph-row-sub">{p.subtitle}</span>
        </Show>
      </span>
      <Show when={p.caption}>
        <span class="ph-row-caption">{p.caption}</span>
      </Show>
      <Show when={p.trailing}>
        <span class="ph-row-trail">{p.trailing}</span>
      </Show>
      <Show when={p.chevron}>
        <ChevronRight size={20} class="ph-row-chevron" aria-hidden="true" />
      </Show>
    </>
  );

  const interactive = () => !!(p.href || p.onClick || p.onLongPress);
  const common = () => ({
    class: 'ph-row',
    'data-selected': p.selected ? '' : undefined,
    'data-dim': p.dim ? '' : undefined,
    'data-wrap': p.wrap ? '' : undefined,
    'data-tone': p.tone,
    'data-strip': p.strip ? '' : undefined,
    'data-two': p.subtitle ? '' : undefined,
    'data-dragging': dragging() ? '' : undefined,
    'aria-current': p.selected ? ('true' as const) : undefined,
    'data-testid': p.testId,
    style: dx() ? { transform: `translateX(${dx()}px)` } : undefined,
    onPointerDown,
    onPointerMove,
    onPointerUp: finish,
    onPointerCancel: finish,
    onContextMenu: (e: MouseEvent) => {
      if (!p.onLongPress) return;
      e.preventDefault();
      // Android WebView fires contextmenu for the same long-press our timer handles.
      if (!longFired) {
        clear();
        fireLong();
      }
    },
    onClick: (e: MouseEvent) => {
      if (longFired) {
        e.preventDefault();
        longFired = false;
        return;
      }
      if (p.disabled) {
        e.preventDefault();
        return;
      }
      p.onClick?.();
    },
  });

  const side = () => (dx() > 0 ? p.swipeStart : dx() < 0 ? p.swipeEnd : undefined);

  return (
    <div ref={wrap} class="ph-row-wrap" role="listitem">
      <Show when={side()}>
        {(action) => (
          <div class="ph-row-swipe" data-side={dx() > 0 ? 'start' : 'end'} data-tone={action().tone} aria-hidden="true">
            <Dynamic component={action().icon} size={22} />
            <span>{action().label}</span>
          </div>
        )}
      </Show>
      <Show
        when={interactive()}
        fallback={
          <div {...common()} role={undefined}>
            {content()}
          </div>
        }
      >
        <Show
          when={p.href}
          fallback={
            <button type="button" disabled={p.disabled} {...common()}>
              {content()}
            </button>
          }
        >
          <a href={`#${p.href}`} aria-disabled={p.disabled ? 'true' : undefined} {...common()}>
            {content()}
          </a>
        </Show>
      </Show>
    </div>
  );
}

export function RowIcon(p: { icon: Icon; tint?: 'neutral' | 'accent' | 'green' | 'amber' | 'red' }) {
  return (
    <span class="ph-row-icon" data-tint={p.tint ?? 'neutral'} aria-hidden="true">
      <Dynamic component={p.icon} size={22} strokeWidth={1.75} />
    </span>
  );
}
