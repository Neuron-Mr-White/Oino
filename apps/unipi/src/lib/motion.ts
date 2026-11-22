/**
 * Exit motion (Z-91, WP-A28; DESIGN.md §1.4 "150–200 ms ease-out").
 *
 * unipi already animated entrances (popover-in, pane-in, toasts-in); removals
 * were instant. This module gives removals the same 160 ms, and nothing at all
 * under `prefers-reduced-motion: reduce` (the global rule in `style.css` drops
 * the keyframes; these helpers keep the element from lingering invisibly).
 *
 *   const fold = createPresence(() => open());
 *   <Show when={fold.mounted()}>
 *     <div data-leaving={fold.leaving() ? '' : undefined}>…</div>
 *   </Show>
 *
 *   const leave = createExit(() => props.onDismiss());
 *   <div data-leaving={leave.leaving() ? '' : undefined}>…</div>
 */
import { createEffect, createSignal, onCleanup, untrack, type Accessor } from 'solid-js';

/** The exit duration. ZCode's collapse used 300 ms; DESIGN caps exits at 200 ms. */
export const EXIT_MS = 160;

/** True when the user asked for reduced motion (web `prefers-reduced-motion`). */
export function prefersReducedMotion(): boolean {
  return typeof matchMedia !== 'undefined' && !!matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** The exit time to use right now: 0 under reduced motion.
 *  The CSS `--exit-ms` owns the duration when it is set (`style.css`), so the
 *  capture harness can slow exits down for frame-by-frame shots; tests and the
 *  plain app fall back to `EXIT_MS`. */
export function exitDurationMs(): number {
  if (prefersReducedMotion()) return 0;
  if (typeof document !== 'undefined' && typeof getComputedStyle === 'function') {
    const raw = getComputedStyle(document.documentElement).getPropertyValue('--exit-ms').trim();
    const ms = raw.endsWith('ms') ? parseFloat(raw) : raw.endsWith('s') ? parseFloat(raw) * 1000 : NaN;
    if (Number.isFinite(ms) && ms >= 0) return ms;
  }
  return EXIT_MS;
}

export type Presence = {
  /** Render the element (`true` while entering, open, and leaving). */
  mounted: Accessor<boolean>;
  /** The element is on its way out: style it with `[data-leaving]`. */
  leaving: Accessor<boolean>;
};

/** Keeps a conditionally rendered element mounted for its exit animation. */
export function createPresence(open: () => boolean, ms?: number): Presence {
  const exit = () => ms ?? exitDurationMs();
  const [mounted, setMounted] = createSignal(open());
  const [leaving, setLeaving] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  createEffect(() => {
    const on = open();
    if (on) {
      stop();
      setLeaving(false);
      setMounted(true);
      return;
    }
    const was = untrack(mounted);
    stop();
    if (!was) return;
    const time = exit();
    // Reduced motion: no exit frame at all (the CSS keyframes are off too).
    if (time === 0) {
      setLeaving(false);
      setMounted(false);
      return;
    }
    setLeaving(true);
    timer = setTimeout(() => {
      timer = undefined;
      setMounted(false);
      setLeaving(false);
    }, time);
  });
  onCleanup(stop);
  return { mounted, leaving };
}

export type Exit = {
  /** Style the element with `[data-leaving]` while `true`. */
  leaving: Accessor<boolean>;
  /** Start the exit; `run` (the owner's close) fires after it (at once under reduced motion). */
  begin: () => void;
};

/** Defers an owner's close by the exit animation. Idempotent. */
export function createExit(run: () => void, ms?: number): Exit {
  const [leaving, setLeaving] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const finish = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    setLeaving(false);
    run();
  };
  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  });
  return {
    leaving,
    begin: () => {
      if (leaving() || timer !== undefined) return;
      const time = ms ?? exitDurationMs();
      if (time === 0) {
        run();
        return;
      }
      setLeaving(true);
      timer = setTimeout(finish, time);
    },
  };
}
