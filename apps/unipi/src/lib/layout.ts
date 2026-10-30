/**
 * Desktop shell layout state (UNI-117).
 *
 * The desktop shell (sidebar, list | detail panes, command palette, cursor
 * context menus, centred dialogs) is on when the window is at least
 * `DESKTOP_MIN_WIDTH` wide AND we are not the phone app (a phone in
 * landscape is ~915px wide but must keep its phone UI). Below that, or on
 * Android, nothing changes: every desktop rule in CSS is scoped under
 * `html[data-shell=desktop]`, so the phone layout is byte-for-byte the old one.
 *
 * List | detail panes: a screen's list renders in the route as usual; its
 * detail is a fixed overlay that carries the class `shell-detail` (chat
 * layer, task page, editor, terminal). On desktop that overlay is placed to
 * the right of the list instead of over it, and a splitter between them
 * resizes the list (`--list-w`, remembered per screen).
 */
import { createRoot, createSignal } from 'solid-js';
import { platform } from './platform';

export const DESKTOP_MIN_WIDTH = 900;
export const SIDEBAR_WIDTH = 224;
/** The list never gets narrower than this, the detail never narrower than `MIN_DETAIL`. */
export const MIN_LIST = 300;
export const MIN_DETAIL = 420;

/** Default list width per screen key (the route's first segment). */
/** One default list width for list | detail screens (UNI-218 F13); the board differs. */
export const DEFAULT_LIST_WIDTH = 400;
const DEFAULT_LIST: Record<string, number | ((main: number) => number)> = {
  sessions: DEFAULT_LIST_WIDTH,
  chat: DEFAULT_LIST_WIDTH,
  files: DEFAULT_LIST_WIDTH,
  // The board's list is its lane columns: give the detail ~560px.
  board: (main) => main - 560,
};

/** Pure: should the desktop shell be on? */
export function wantsDesktopShell(width: number, mode: string, hasMatchMedia: boolean): boolean {
  return hasMatchMedia && mode !== 'tauri-mobile' && width >= DESKTOP_MIN_WIDTH;
}

/** Pure: clamp a list width so both panes keep their minimums. */
export function clampListWidth(width: number, main: number): number {
  const max = Math.max(MIN_LIST, main - MIN_DETAIL);
  return Math.round(Math.min(Math.max(width, MIN_LIST), max));
}

/** Pure: default list width for a screen in a main area of `main` px. */
export function defaultListWidth(screen: string, main: number): number {
  const value = DEFAULT_LIST[screen] ?? DEFAULT_LIST_WIDTH;
  return clampListWidth(typeof value === 'function' ? value(main) : value, main);
}

const LIST_KEY = 'unipi.shell.listWidth';

function readWidths(): Record<string, number> {
  try {
    const parsed = JSON.parse(localStorage.getItem(LIST_KEY) ?? '{}') as unknown;
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, number>) : {};
  } catch {
    return {};
  }
}

function createLayout() {
  const hasMatchMedia = typeof window !== 'undefined' && typeof window.matchMedia === 'function';
  const compute = () => wantsDesktopShell(typeof window !== 'undefined' ? window.innerWidth : 0, platform().mode, hasMatchMedia);
  const [desktop, setDesktop] = createSignal(compute());
  const [widths, setWidths] = createSignal<Record<string, number>>(readWidths());
  const [viewport, setViewport] = createSignal(typeof window !== 'undefined' ? window.innerWidth : 0);
  if (hasMatchMedia) {
    const query = window.matchMedia(`(min-width: ${DESKTOP_MIN_WIDTH}px)`);
    const update = () => setDesktop(compute());
    query.addEventListener?.('change', update);
    window.addEventListener('resize', () => {
      setViewport(window.innerWidth);
      update();
    });
  }
  const mainWidth = () => Math.max(0, viewport() - SIDEBAR_WIDTH);
  return {
    /** Desktop shell on (sidebar, panes, palette)? */
    desktop,
    /** The list width (px) for a screen, clamped to the current window. */
    listWidth: (screen: string) => {
      const saved = widths()[screen];
      return saved ? clampListWidth(saved, mainWidth()) : defaultListWidth(screen, mainWidth());
    },
    setListWidth: (screen: string, width: number) => {
      const next = { ...widths(), [screen]: clampListWidth(width, mainWidth()) };
      setWidths(next);
      try {
        localStorage.setItem(LIST_KEY, JSON.stringify(next));
      } catch {
        // storage full / blocked: width just isn't remembered
      }
    },
    resetListWidth: (screen: string) => {
      const next = { ...widths() };
      delete next[screen];
      setWidths(next);
      try {
        localStorage.setItem(LIST_KEY, JSON.stringify(next));
      } catch {
        // ignore
      }
    },
  };
}

let shared: ReturnType<typeof createLayout> | undefined;
export function layout(): ReturnType<typeof createLayout> {
  if (!shared) shared = createRoot(() => createLayout());
  return shared;
}
/** Test hook. */
export function resetLayout(): void {
  shared = undefined;
}

/** Shorthand: is the desktop shell on? */
export function desktopShell(): boolean {
  return layout().desktop();
}

/**
 * Where a context menu should open. On the desktop shell a right-click
 * opens it at the cursor (a zero-size rect at the pointer); on a phone the
 * long-press menu stays anchored to the pressed element, as before.
 */
export function menuAnchor(event: MouseEvent, element: Element): DOMRect {
  const rect = element.getBoundingClientRect();
  const atPointer = desktopShell() && (event.clientX !== 0 || event.clientY !== 0);
  if (!atPointer || typeof DOMRect === 'undefined') return rect;
  return new DOMRect(event.clientX, event.clientY, 0, 0);
}
