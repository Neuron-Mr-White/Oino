/**
 * w2-ui2 T1: "the first frame has been painted" — a signal the launch uses to mount the heavy
 * part of the screen (the open chat: timeline, composer, panes) one task AFTER the shell and the
 * sidebar rows, so the rows paint without waiting for it. Set once per page load; later route
 * changes mount at once. Without a frame (a hidden window, tests) a timer sets it anyway.
 */
import { createRoot, createSignal } from 'solid-js';

/** The longest the chat waits for a frame that never comes. */
export const FIRST_PAINT_FALLBACK_MS = 250;

const [done, setDone] = createRoot(() => createSignal(import.meta.env?.MODE === 'test'));
let started = false;

/** True once a frame has been painted since the page loaded (reading it starts the wait). */
export function firstPaintDone(): boolean {
  if (!started && !done()) {
    started = true;
    const finish = () => setDone(true);
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => setTimeout(finish, 0));
    setTimeout(finish, FIRST_PAINT_FALLBACK_MS);
  }
  return done();
}

/** Tests: the state a fresh page load starts in. */
export function resetFirstPaint(done_ = false): void {
  started = false;
  setDone(done_);
}
