/**
 * Files tree refresh (app spec WP-A20, Z-101): "watchers on expanded dirs —
 * poll-free via `fs` events if H7 offers them, else refresh on focus".
 *
 * The host protocol (H7) has no filesystem watch event yet (a `git_watch`
 * hub event arrives with A15), so the default here is **refresh on focus**:
 * the window regaining focus, the tab becoming visible, and the pane tab
 * body being un-hidden (the panes container keeps bodies mounted; the
 * `hidden` attribute flips when the tab is activated). No timers, no
 * polling. When a watch event lands later, swap the signal source.
 *
 * `mutated()` is the explicit bump after the tree itself changes something
 * (rename/delete/upload/new file) so the list reflects its own writes
 * without waiting for a focus change.
 */
import { createSignal, onCleanup } from 'solid-js';

export type WatchSources = {
  /** The element whose `hidden` attribute flips when this pane tab shows (`.pane-tab-body`). */
  body?: () => HTMLElement | undefined;
};

/**
 * A focus-driven dir refresh: `dirty()` becomes true whenever the tree may
 * be stale; the component re-lists the (expanded) dirs and clears it.
 */
export function createDirWatch(sources: WatchSources = {}) {
  const [dirty, setDirty] = createSignal(false);
  const bump = () => setDirty(true);

  window.addEventListener('focus', bump);
  document.addEventListener('visibilitychange', bump);
  onCleanup(() => {
    window.removeEventListener('focus', bump);
    document.removeEventListener('visibilitychange', bump);
  });

  let observer: MutationObserver | undefined;
  const body = sources.body?.();
  if (body) {
    observer = new MutationObserver(bump);
    observer.observe(body, { attributes: true, attributeFilter: ['hidden'] });
    onCleanup(() => observer?.disconnect());
  }

  return {
    /** True while a refresh is due (the tree re-lists, then `clear()`s). */
    dirty,
    clear: () => setDirty(false),
    /** Something the tree did itself changed the filesystem. */
    mutated: bump,
  };
}

export type DirWatch = ReturnType<typeof createDirWatch>;
