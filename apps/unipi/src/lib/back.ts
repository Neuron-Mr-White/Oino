/**
 * Android Back for in-page overlays (history panel, composer, the
 * terminal's session picker). Tauri's app plugin calls our listener
 * *instead of* WebView.goBack() while one is registered, so the
 * handlers form a stack: Back runs only the top-most handler, and
 * when the stack is empty the listener unregisters, which restores
 * the default (history back: /term → Home).
 *
 * Outside Tauri (tests, browser dev) Escape plays the role of Back.
 *
 * Web build (the kanboard daemon serves it, UNI-117): the browser's own
 * Back button must close the top overlay too, not leave the app. Each
 * pushed handler adds a same-URL history entry (marked in its state); a
 * popstate that leaves the top entry runs the top handler. A handler
 * popped by the app itself (closed with ✕, Esc…) takes its entry back
 * off with history.back() — only while that entry is still the current
 * one, so a route change meanwhile is never undone.
 */
import { can, isTauri, isWeb } from './platform';
type Handler = () => void;

const stack: Handler[] = [];
/** Web mode: the history marker of each stacked handler (same order as `stack`). */
const marks = new Map<Handler, number>();
const MARK = '__unipiBack';
let nextMark = 1;
/** history.back() calls of our own whose popstate must be ignored. */
let selfBacks = 0;
let popListening = false;
/** Web mode: the entry a just-popped overlay will give back (see pop). */
let pendingBack: number | undefined;

function markOf(state: unknown): number | undefined {
  const value = (state as Record<string, unknown> | null)?.[MARK];
  return typeof value === 'number' ? value : undefined;
}

function onPopState(event: PopStateEvent): void {
  if (selfBacks > 0) {
    selfBacks -= 1;
    return;
  }
  const top = stack.at(-1);
  if (!top) return;
  const mark = marks.get(top);
  // Still on (or moved forward onto) the top overlay's entry: nothing left.
  if (mark === undefined || markOf(event.state) === mark) return;
  marks.delete(top); // its entry is already gone: pop must not go back again
  top();
  // The handler kept its overlay (a "Discard your draft?" guard): give it
  // a fresh entry, so the next Back asks again instead of leaving the app.
  if (stack.at(-1) === top) {
    const again = nextMark++;
    marks.set(top, again);
    try {
      window.history.pushState({ ...(window.history.state ?? {}), [MARK]: again }, '');
    } catch {
      marks.delete(top);
    }
  }
}

function webHistory(): boolean {
  return isWeb() && typeof window !== 'undefined' && typeof window.history?.pushState === 'function';
}
let unregister: (() => void) | undefined;
let registering = false;

/**
 * Phone spec §1.3: what Back does when no overlay is open (the phone shell
 * sets it: route → hierarchical parent, then Chats, then background). While
 * it is set the native listener stays registered even with an empty stack.
 */
let rootBack: Handler | undefined;

function runTop(): void {
  const top = stack.at(-1);
  if (top) top();
  else rootBack?.();
}

/** Installs (or clears with `undefined`) the route-level Back of the phone shell. */
export function setRootBack(handler: Handler | undefined): void {
  rootBack = handler;
  if (handler) void ensureListener();
  else if (stack.length === 0) releaseListener();
}

function onKey(event: KeyboardEvent): void {
  if (event.key === 'Escape' && stack.length > 0) runTop();
}

async function ensureListener(): Promise<void> {
  if (unregister || registering) return;
  registering = true;
  window.addEventListener('keydown', onKey);
  let off: (() => void) | undefined;
  if (isTauri() && can('backButton')) {
    try {
      const { onBackButtonPress } = await import('@tauri-apps/api/app');
      const listener = await onBackButtonPress(() => runTop());
      off = () => void listener.unregister();
    } catch (error) {
      console.warn('[back] listener unavailable', error);
    }
  }
  registering = false;
  unregister = () => {
    window.removeEventListener('keydown', onKey);
    off?.();
  };
  // Everything popped while we were registering: undo right away.
  if (stack.length === 0 && !rootBack) releaseListener();
}

function releaseListener(): void {
  if (registering) return; // ensureListener() releases once done
  if (rootBack) return; // the phone shell's route Back keeps it
  unregister?.();
  unregister = undefined;
}

/** Pushes a Back handler; call the returned function to pop it. */
export function pushBack(handler: Handler): () => void {
  stack.push(handler);
  void ensureListener();
  if (webHistory()) {
    if (!popListening) {
      window.addEventListener('popstate', onPopState);
      popListening = true;
    }
    const mark = nextMark++;
    marks.set(handler, mark);
    try {
      // An overlay that just closed (a menu whose item opens a sheet) hasn't
      // given its entry back yet: reuse it instead of back()+push, which
      // would race (the async back would pop the new overlay's entry).
      if (pendingBack !== undefined && markOf(window.history.state) === pendingBack) {
        pendingBack = undefined;
        window.history.replaceState({ ...(window.history.state ?? {}), [MARK]: mark }, '');
      } else window.history.pushState({ ...(window.history.state ?? {}), [MARK]: mark }, '');
    } catch {
      marks.delete(handler);
    }
  }
  let popped = false;
  return () => {
    if (popped) return;
    popped = true;
    const index = stack.lastIndexOf(handler);
    if (index >= 0) stack.splice(index, 1);
    const mark = marks.get(handler);
    marks.delete(handler);
    if (mark !== undefined && markOf(window.history.state) === mark) {
      pendingBack = mark;
      queueMicrotask(() => {
        if (pendingBack !== mark) return; // reused by a push meanwhile
        pendingBack = undefined;
        if (markOf(window.history.state) !== mark) return;
        selfBacks += 1;
        window.history.back();
      });
    }
    if (stack.length === 0) releaseListener();
  };
}

/** Test hook: how many handlers are on the stack. */
export function backDepth(): number {
  return stack.length;
}
