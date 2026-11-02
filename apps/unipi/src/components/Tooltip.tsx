/**
 * The one tooltip (Z-116, WP-A28): a 300 ms shared hover tip with a
 * per-trigger timer, shown for every icon-only control that carries an
 * accessible name (plain English, lucide icons only, no emoji) and for any
 * element with `data-tip`. Nothing here adds a focus ring or a 44 px target —
 * those stay unipi's own (`ui.css`).
 *
 *   <button aria-label="New chat" data-tip="New chat">…icon…</button>
 *   <Tooltip label="Delete task"><button disabled aria-label="Delete task">…</button></Tooltip>
 *
 * The attribute form is enough for a normal control; the `<Tooltip>` wrapper
 * exists for **disabled** triggers — Chromium dispatches no pointer events to
 * a disabled form control, so the wrapper (an inline-flex box around it)
 * receives the hover and the tip still works. `TooltipHost` mounts once in the
 * shell (`AppShell`) and renders the single bubble; a trigger never renders
 * its own.
 */
import { createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { isTauriMobile } from '../lib/platform';

/** Hover delay (ZCode `docs/ui/sidebar-tooltips.md`: 300 ms, per trigger). */
export const TIP_DELAY_MS = 300;
/** The bubble's exit fade; CSS holds it at 0 under reduced motion. */
export const TIP_EXIT_MS = 120;
/** Gap between the trigger and the bubble. */
export const TIP_GAP = 6;
/** The bubble's widest size (centring uses it; short tips sit inside it). */
export const TIP_WIDTH = 260;
/** Distance kept from the viewport edge. */
const EDGE = 8;
const TIP_ID = 'unipi-tooltip';

/** The interactive elements a tooltip may attach to. */
const TRIGGER = '[data-tip], button, a[href], [role="button"], [role="tab"], [role="menuitem"], [role="switch"], [role="checkbox"]';

/** True when the element shows only graphics (an icon, a sprite): no text of its own. */
export function isIconOnly(el: Element): boolean {
  if ((el.textContent ?? '').trim() !== '') return false;
  return !!el.querySelector('svg, img, canvas');
}

function labelledByText(el: Element): string | undefined {
  const ids = (el.getAttribute('aria-labelledby') ?? '').split(/\s+/).filter(Boolean);
  if (!ids.length) return undefined;
  const doc = el.ownerDocument;
  const text = ids
    .map((id) => doc.getElementById(id)?.textContent?.trim() ?? '')
    .filter(Boolean)
    .join(' ');
  return text || undefined;
}

/** The element's accessible name, without falling back to `title` (a native
 *  tooltip is not a name we accept — the sweep gives controls real labels). */
export function accessibleName(el: Element): string | undefined {
  const label = el.getAttribute('aria-label')?.trim();
  if (label) return label;
  const labelled = labelledByText(el);
  if (labelled) return labelled;
  const text = (el.textContent ?? '').replace(/\s+/g, ' ').trim();
  return text || undefined;
}

/** Emoji / pictographs are not allowed in UI copy (QA#10) — a tip in one is a bug. */
const EMOJI = /[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{FE0F}]/u;

/** True for a tip we are willing to show: real words, no emoji, no bare glyph. */
export function isPlainText(text: string): boolean {
  const t = text.trim();
  return t.length >= 2 && /[A-Za-z0-9]/.test(t) && !EMOJI.test(t);
}

/**
 * The tip text the host would show for this element, or `undefined` (this is
 * also what `a11y.test.tsx` asserts over every icon-only control):
 * `data-tip` always wins, then an enclosing `<Tooltip>`'s label (it says
 * something better than the control's name, e.g. why Send is disabled),
 * then an icon-only control's accessible name. Text-bearing elements get no
 * automatic tip.
 */
export function tooltipTextFor(el: Element): string | undefined {
  const clean = (v: string | null | undefined) => v?.replace(/\s+/g, ' ').trim() || undefined;
  const own = clean(el.getAttribute('data-tip'));
  const wrapped = clean(el.closest('.tip')?.getAttribute('data-tip'));
  const text = own ?? wrapped ?? (isIconOnly(el) ? accessibleName(el) : undefined);
  return text && isPlainText(text) ? text : undefined;
}

export type TipState = { text: string; el: Element; rect: DOMRect; leaving?: boolean };

const [tip, setTip] = createSignal<TipState | undefined>();
/** Test/support seam: the tip currently shown. */
export const currentTip = tip;

/**
 * The one bubble. Mounted once (`AppShell`), listens on the document so
 * components need no handler of their own.
 */
/** Phone shell on a touch screen (the app, or a narrow window on a coarse pointer): tooltips stay off. */
export function touchPhone(): boolean {
  if (typeof document === 'undefined' || document.documentElement.dataset.shell !== 'phone') return false;
  return isTauriMobile() || (typeof matchMedia !== 'undefined' && !!matchMedia('(pointer: coarse)').matches);
}

export function TooltipHost() {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let exit: ReturnType<typeof setTimeout> | undefined;
  let trigger: Element | undefined;
  /** The trigger's own `aria-describedby`, restored when the tip hides. */
  let described: string | null = null;
  /** A native `title` is hidden while the shared bubble shows (one tooltip, not two). */
  let savedTitle: string | null = null;

  const reducedMotion = () => typeof matchMedia !== 'undefined' && !!matchMedia('(prefers-reduced-motion: reduce)').matches;

  const clearTimer = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };

  const detach = () => {
    if (!trigger) return;
    if (described === null) trigger.removeAttribute('aria-describedby');
    else trigger.setAttribute('aria-describedby', described);
    if (savedTitle !== null) trigger.setAttribute('title', savedTitle);
    trigger = undefined;
    described = null;
    savedTitle = null;
  };

  const hide = (fadeOut = true) => {
    clearTimer();
    detach();
    const shown = tip();
    if (!shown) return;
    if (!fadeOut || reducedMotion()) {
      setTip(undefined);
      return;
    }
    // Leave the bubble one short fade, then drop it.
    setTip({ ...shown, leaving: true });
    if (exit !== undefined) clearTimeout(exit);
    exit = setTimeout(() => setTip(undefined), TIP_EXIT_MS);
  };

  const show = (el: Element, text: string) => {
    if (exit !== undefined) clearTimeout(exit);
    exit = undefined;
    trigger = el;
    described = el.getAttribute('aria-describedby');
    el.setAttribute('aria-describedby', TIP_ID);
    savedTitle = el.getAttribute('title');
    if (savedTitle !== null) el.removeAttribute('title');
    setTip({ text, el, rect: el.getBoundingClientRect() });
  };

  const triggerOf = (target: EventTarget | null): Element | undefined => {
    const el = target as Element | null;
    return el?.closest?.(TRIGGER) ?? undefined;
  };

  const onPointerOver = (event: PointerEvent) => {
    // Touch taps act at once (ZCode SBT04): no hover tip, no 300 ms wait.
    if (event.pointerType === 'touch' || touchPhone()) return;
    const el = triggerOf(event.target);
    if (!el) return hide();
    const text = tooltipTextFor(el);
    if (!text) return hide();
    if (el === trigger) return;
    // A new trigger always waits the full delay (no shared "skip delay" state).
    hide(false);
    trigger = el;
    clearTimer();
    timer = setTimeout(() => {
      timer = undefined;
      if (trigger === el && el.isConnected) show(el, text);
    }, TIP_DELAY_MS);
  };

  const onPointerOut = (event: PointerEvent) => {
    if (!trigger) return;
    const to = event.relatedTarget as Node | null;
    if (to && trigger.contains(to)) return;
    hide();
  };

  const onFocusIn = (event: FocusEvent) => {
    // Phone spec §3.13 (PH-37): a tap focuses the button — no tooltip for touch on the phone shell.
    if (touchPhone()) return;
    const el = triggerOf(event.target);
    if (!el) return;
    const text = tooltipTextFor(el);
    if (!text) return;
    // Keyboard focus needs no delay; the ring already says "you are here".
    clearTimer();
    show(el, text);
  };

  const onFocusOut = () => hide();
  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') hide();
  };
  const onScroll = () => hide(false);
  const onPress = () => hide(false);

  onMount(() => {
    document.addEventListener('pointerover', onPointerOver, true);
    document.addEventListener('pointerout', onPointerOut, true);
    document.addEventListener('focusin', onFocusIn, true);
    document.addEventListener('focusout', onFocusOut, true);
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('pointerdown', onPress, true);
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', onScroll);
    onCleanup(() => {
      document.removeEventListener('pointerover', onPointerOver, true);
      document.removeEventListener('pointerout', onPointerOut, true);
      document.removeEventListener('focusin', onFocusIn, true);
      document.removeEventListener('focusout', onFocusOut, true);
      document.removeEventListener('keydown', onKey, true);
      document.removeEventListener('pointerdown', onPress, true);
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', onScroll);
      clearTimer();
      if (exit !== undefined) clearTimeout(exit);
      detach();
      setTip(undefined);
    });
  });

  /** Centred on the trigger, kept 8 px inside the viewport. */
  const leftFor = (rect: DOMRect): number => {
    const half = TIP_WIDTH / 2;
    return Math.round(Math.max(EDGE + half, Math.min(rect.left + rect.width / 2, window.innerWidth - half - EDGE)) - half);
  };

  return (
    <Show when={tip()}>
      {(shown) => {
        const above = () => shown().rect.top > 36;
        return (
          <Portal>
            <div
              id={TIP_ID}
              role="tooltip"
              class="tip-bubble"
              data-side={above() ? 'top' : 'bottom'}
              data-leaving={shown().leaving ? '' : undefined}
              data-testid="tooltip"
              style={{
                left: `${leftFor(shown().rect)}px`,
                top: `${above() ? shown().rect.top - TIP_GAP : shown().rect.bottom + TIP_GAP}px`,
              }}
            >
              {shown().text}
            </div>
          </Portal>
        );
      }}
    </Show>
  );
}

/**
 * Wrapper for a trigger the host cannot hover itself — a disabled control
 * (Chromium sends it no pointer events). The inline-flex box keeps the
 * trigger's size, so headers and rows don't move.
 */
export function Tooltip(props: { label: string; children: JSX.Element; class?: string }) {
  return (
    <span class={`tip ${props.class ?? ''}`} data-tip={props.label}>
      {props.children}
    </span>
  );
}
