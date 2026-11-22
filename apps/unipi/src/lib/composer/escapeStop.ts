/**
 * Esc stops the run (WP-A11, Z-55 — ruling: ZCode's guard): from **any**
 * focus in the conversation, not just the composer, unless a dialog or
 * popup consumed the key first (they run before this listener in the bubble
 * order and call `preventDefault()` — a docked dialog consumes Esc, an open
 * chevron menu closes, …). IME composition never stops anything.
 *
 * `installEscapeStop` wires the predicate to one conversation root; it is
 * inert while no run is going and while the session is read-only.
 */

export type EscapeStopEvent = Pick<KeyboardEvent, 'key' | 'defaultPrevented' | 'isComposing'>;

/** Whether this keydown should stop the run (Z-55 consumption rules). */
export function escapeShouldStop(ev: EscapeStopEvent, running: boolean): boolean {
  if (ev.key !== 'Escape') return false;
  if (ev.defaultPrevented) return false; // a dialog / popup consumed it
  if (ev.isComposing) return false; // IME is still holding the key
  return running;
}

/**
 * Stops via Esc on `root` (the conversation element; events from anything
 * focused inside it bubble here after inner consumers). Returns the cleanup.
 */
export function installEscapeStop(opts: {
  root: HTMLElement;
  running(): boolean;
  stop(): void;
}): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (!escapeShouldStop(e, opts.running())) return;
    e.preventDefault(); // deeper roots (document) must not see a free Esc
    opts.stop();
  };
  opts.root.addEventListener('keydown', onKey);
  return () => opts.root.removeEventListener('keydown', onKey);
}
