/**
 * The one app-wide toast stack (app spec §3.3, WP-A19; Z-85): confirmations,
 * extension `notify` effects and board actions surface here — compact,
 * stacked bottom-right above the composer, one optional action, identical
 * toasts fold ×N, at most 3 visible, hover pauses the timer, click dismisses,
 * and dismissal exits through a short fade (none under reduced motion).
 *
 * Timings: info/success 3 s, warning 5 s, error 8 s, and an action buys 4 s more.
 *
 *   const stack = toasts();
 *   stack.push('Committed abc1234 · fix: width');
 *   stack.push('Task archived', 'info', { label: 'Undo', run: () => … });
 */
import { createSignal } from 'solid-js';

export type ToastKind = 'info' | 'success' | 'warning' | 'error';
export type ToastAction = { label: string; run: () => void | Promise<void> };
export type Toast = { id: number; kind: ToastKind; message: string; action?: ToastAction; count?: number; leaving?: boolean };

/** Info/success toasts auto-dismiss after 3 s. */
export const TOAST_INFO_MS = 3000;
/** Warnings stay longer: 5 s. */
export const TOAST_WARNING_MS = 5000;
/** Errors stay longest so they can be read, then auto-dismiss: 8 s. */
export const TOAST_ERROR_MS = 8000;
/** An action (Undo / Open) buys 4 s more. */
export const TOAST_ACTION_EXTRA_MS = 4000;
/** The exit fade (≤ 200 ms; reduced motion drops it in CSS). */
export const TOAST_EXIT_MS = 180;
/** At most 3 visible at once. */
export const TOAST_MAX_VISIBLE = 3;

/** One toast's lifetime in ms. Pure. */
export function toastTtl(kind: ToastKind, hasAction = false): number {
  const base = kind === 'error' ? TOAST_ERROR_MS : kind === 'warning' ? TOAST_WARNING_MS : TOAST_INFO_MS;
  return base + (hasAction ? TOAST_ACTION_EXTRA_MS : 0);
}

export type ToastTimers = {
  set: (ms: number, cb: () => void) => unknown;
  clear: (handle: unknown) => void;
};
const realTimers: ToastTimers = {
  set: (ms, cb) => setTimeout(cb, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export type ToastStack = {
  readonly list: () => Toast[];
  /** Shows a toast; an identical one (same kind + message, no action) folds into ×N. */
  push(message: string, kind?: ToastKind, action?: ToastAction): number;
  /** Starts the exit fade, then drops the toast. */
  dismiss(id: number): void;
  /** Freezes a toast's timer (hover). */
  pause(id: number): void;
  /** Resumes a paused toast with its remaining time. */
  resume(id: number): void;
  /** Test/support: drop everything at once (no fade). */
  clear(): void;
};

/** The stack's state machine over injectable timers (tests pass fakes). */
export function createToastStack(timers: ToastTimers = realTimers, now: () => number = Date.now): ToastStack {
  const [list, setList] = createSignal<Toast[]>([]);
  let nextId = 0;
  const ttl = new Map<number, unknown>();
  const exit = new Map<number, unknown>();
  const deadline = new Map<number, number>();
  const paused = new Map<number, number>();

  const drop = (id: number) => {
    timers.clear(ttl.get(id));
    timers.clear(exit.get(id));
    ttl.delete(id);
    exit.delete(id);
    deadline.delete(id);
    paused.delete(id);
    setList((current) => current.filter((t) => t.id !== id));
  };

  /** Starts the exit fade, then drops the toast (manual or timed out). */
  function dismiss(id: number): void {
    const toast = list().find((t) => t.id === id);
    if (!toast) return;
    timers.clear(ttl.get(id));
    ttl.delete(id);
    deadline.delete(id);
    paused.delete(id);
    if (toast.leaving) return;
    setList((current) => current.map((t) => (t.id === id ? { ...t, leaving: true } : t)));
    exit.set(id, timers.set(TOAST_EXIT_MS, () => drop(id)));
  }

  const arm = (toast: Toast, ms: number) => {
    timers.clear(ttl.get(toast.id));
    deadline.set(toast.id, now() + ms);
    // Auto-dismissal exits through the same fade as a manual one.
    ttl.set(toast.id, timers.set(ms, () => dismiss(toast.id)));
  };

  return {
    list: () => list(),
    push(message, kind = 'info', action) {
      // Identical toasts fold into ×N and get a fresh timer.
      const same = !action ? list().find((t) => !t.action && !t.leaving && t.kind === kind && t.message === message) : undefined;
      if (same) {
        setList((current) => current.map((t) => (t.id === same.id ? { ...t, count: (t.count ?? 1) + 1 } : t)));
        if (toastTtl(kind) > 0) arm(same, toastTtl(kind));
        return same.id;
      }
      const entry: Toast = { id: ++nextId, kind, message, action };
      setList((current) => {
        const next = [...current, entry];
        // Oldest beyond 3 go at once (their timers too).
        while (next.length > TOAST_MAX_VISIBLE) {
          const gone = next.shift()!;
          timers.clear(ttl.get(gone.id));
          ttl.delete(gone.id);
          deadline.delete(gone.id);
          paused.delete(gone.id);
        }
        return next;
      });
      if (toastTtl(kind, !!action) > 0) arm(entry, toastTtl(kind, !!action));
      return entry.id;
    },
    dismiss,
    pause(id) {
      const toast = list().find((t) => t.id === id);
      if (!toast || toast.leaving || !ttl.has(id)) return;
      const remaining = Math.max(0, (deadline.get(id) ?? now()) - now());
      timers.clear(ttl.get(id));
      ttl.delete(id);
      paused.set(id, remaining);
    },
    resume(id) {
      const toast = list().find((t) => t.id === id);
      if (!toast || toast.leaving) return;
      const remaining = paused.get(id);
      if (remaining === undefined) return;
      paused.delete(id);
      arm(toast, remaining);
    },
    clear() {
      for (const id of [...ttl.keys(), ...exit.keys()]) drop(id);
      setList([]);
    },
  };
}

type GlobalWithToasts = typeof globalThis & { __unipiToasts?: ToastStack };

let shared: ToastStack | undefined;
/** The one app-wide stack (`components/Toasts.tsx` renders it). */
export function toasts(): ToastStack {
  if (!shared) shared = createToastStack();
  (globalThis as GlobalWithToasts).__unipiToasts = shared;
  return shared;
}
/** Tests: replace the shared stack. */
export function setToasts(stack: ToastStack | undefined): void {
  shared = stack;
  (globalThis as GlobalWithToasts).__unipiToasts = stack;
}
