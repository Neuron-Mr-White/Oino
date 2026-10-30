/**
 * Shell geometry (app spec §3.7, WP-A3; UNI-117 for the desktop switch).
 *
 * The desktop shell (sidebar · conversation frame · side pane · bottom panel)
 * is on when the window is at least `DESKTOP_MIN_WIDTH` wide AND we are not
 * the phone app (a phone in landscape is ~915 px wide but keeps its phone
 * UI). Every desktop rule in CSS is scoped under `html[data-shell=desktop]`.
 *
 * Geometry (ZCode `workspace-layout.md`, `workspace-resize-auto-collapse.md`,
 * RULINGS Z-95), all pure and tested in `layout.test.ts`:
 * - sidebar 264 px (220–420), Cmd/Ctrl+B collapses it;
 * - side pane 480 px (min 320); it **pushes** the chat only when the window is
 *   ≥ 1280 px, below that it opens as an **overlay** over the conversation;
 * - bottom panel 280 px (min 120), Cmd/Ctrl+J;
 * - on window resize (only then, never when the user opens a pane) the side
 *   pane auto-collapses when the chat column would be < 480 px (or the window
 *   drops into overlay mode), the sidebar when < 360 px; both come back when
 *   the space returns unless the user closed them meanwhile.
 */
import { createRoot, createSignal } from 'solid-js';
import { platform } from './platform';

export const DESKTOP_MIN_WIDTH = 900;

export const SIDEBAR_DEFAULT = 264;
export const SIDEBAR_MIN = 220;
export const SIDEBAR_MAX = 420;
/** Kept for the legacy browser side sheet (BrowserSide, wave 3 replaces it). */
export const SIDEBAR_WIDTH = SIDEBAR_DEFAULT;
export const SIDE_DEFAULT = 480;
export const SIDE_MIN = 320;
export const BOTTOM_DEFAULT = 280;
export const BOTTOM_MIN = 120;
/** The conversation never gets shorter than this when the bottom panel grows. */
export const CONV_MIN_HEIGHT = 200;
/** Auto-collapse thresholds on the chat column width (Z-3). */
export const CHAT_MIN_WITH_SIDE = 480;
export const CHAT_MIN_WITH_SIDEBAR = 360;
/** Window width from which the side pane pushes the chat instead of overlaying it (Z-95). */
export const PUSH_MIN = 1280;

export type SideMode = 'push' | 'overlay';

/** Pure: should the desktop shell be on? */
export function wantsDesktopShell(width: number, mode: string, hasMatchMedia: boolean): boolean {
  return hasMatchMedia && mode !== 'tauri-mobile' && width >= DESKTOP_MIN_WIDTH;
}

/** Pure: push or overlay for a window this wide. */
export function sideMode(viewport: number): SideMode {
  return viewport >= PUSH_MIN ? 'push' : 'overlay';
}

export type ShellPrefs = {
  sidebarOpen: boolean;
  sidebarW: number;
  sideOpen: boolean;
  sideW: number;
  bottomOpen: boolean;
  bottomH: number;
};

export const DEFAULT_PREFS: ShellPrefs = {
  sidebarOpen: true,
  sidebarW: SIDEBAR_DEFAULT,
  sideOpen: false,
  sideW: SIDE_DEFAULT,
  bottomOpen: false,
  bottomH: BOTTOM_DEFAULT,
};

/** Panes hidden by the resize rule (not by the user). */
export type AutoCollapsed = { sidebar: boolean; side: boolean };

/** Pure: the chat column width for this window and these visible panes. */
export function chatWidth(viewport: number, sidebar: number, side: number): number {
  const pushed = sideMode(viewport) === 'push' ? side : 0;
  return viewport - sidebar - pushed;
}

/**
 * Pure: the resize rule (Z-3). Called on window resize only. Collapses the
 * side pane first (chat < 480 px, or overlay mode), then the sidebar
 * (chat < 360 px); restores what it collapsed once the space is back.
 */
export function autoCollapse(prefs: ShellPrefs, auto: AutoCollapsed, viewport: number): AutoCollapsed {
  let sidebar = auto.sidebar && prefs.sidebarOpen;
  let side = auto.side && prefs.sideOpen;
  let sbOn = prefs.sidebarOpen && !sidebar;
  let spOn = prefs.sideOpen && !side;
  const push = sideMode(viewport) === 'push';
  const chat = (sb: boolean, sp: boolean) => chatWidth(viewport, sb ? prefs.sidebarW : 0, sp ? prefs.sideW : 0);
  if (spOn && (!push || chat(sbOn, true) < CHAT_MIN_WITH_SIDE)) {
    side = true;
    spOn = false;
  }
  if (sbOn && chat(true, spOn) < CHAT_MIN_WITH_SIDEBAR) {
    sidebar = true;
    sbOn = false;
  }
  if (sidebar && chat(true, spOn) >= CHAT_MIN_WITH_SIDEBAR) {
    sidebar = false;
    sbOn = true;
  }
  if (side && push && chat(sbOn, true) >= CHAT_MIN_WITH_SIDE) side = false;
  return { sidebar, side };
}

/** Pure: sidebar width within 220–420 and leaving the chat its 360 px. */
export function clampSidebar(width: number, viewport: number): number {
  const room = viewport - CHAT_MIN_WITH_SIDEBAR;
  return Math.round(Math.max(SIDEBAR_MIN, Math.min(width, SIDEBAR_MAX, room)));
}

/** Pure: side pane width ≥ 320; in push mode it leaves the chat 480 px. */
export function clampSide(width: number, viewport: number, sidebar: number): number {
  const room = sideMode(viewport) === 'push' ? viewport - sidebar - CHAT_MIN_WITH_SIDE : viewport - sidebar - 48;
  return Math.round(Math.max(SIDE_MIN, Math.min(width, room)));
}

/** Pure: bottom panel height ≥ 120 and leaving the conversation 200 px. */
export function clampBottom(height: number, frame: number): number {
  return Math.round(Math.max(BOTTOM_MIN, Math.min(height, frame - CONV_MIN_HEIGHT)));
}

const PREFS_KEY = 'unipi.shell.layout';

export function readPrefs(raw: string | null): ShellPrefs {
  try {
    const parsed = JSON.parse(raw ?? '{}') as Partial<ShellPrefs> | null;
    if (!parsed || typeof parsed !== 'object') return { ...DEFAULT_PREFS };
    const out = { ...DEFAULT_PREFS };
    for (const key of Object.keys(DEFAULT_PREFS) as (keyof ShellPrefs)[]) {
      const value = parsed[key];
      if (typeof value === typeof DEFAULT_PREFS[key]) (out as Record<string, unknown>)[key] = value;
    }
    return out;
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

let desktopOverride: boolean | undefined;
/** Test hook: force the desktop (true) or phone (false) shell; `undefined` = measure. */
export function setDesktopOverride(on: boolean | undefined): void {
  desktopOverride = on;
  resetLayout();
}

function storage(): Storage | undefined {
  try {
    return typeof localStorage === 'undefined' ? undefined : localStorage;
  } catch {
    return undefined;
  }
}

function createLayout() {
  const hasWindow = typeof window !== 'undefined';
  const hasMatchMedia = hasWindow && typeof window.matchMedia === 'function';
  const vw = () => (hasWindow ? window.innerWidth : 0);
  const compute = () => desktopOverride ?? wantsDesktopShell(vw(), platform().mode, hasMatchMedia);
  const [desktop, setDesktop] = createSignal(compute());
  const [viewport, setViewport] = createSignal(vw());
  const [prefs, setPrefsSignal] = createSignal<ShellPrefs>(readPrefs(storage()?.getItem(PREFS_KEY) ?? null));
  const [auto, setAuto] = createSignal<AutoCollapsed>(autoCollapse(prefs(), { sidebar: false, side: false }, vw()));
  const [fullView, setFullViewSignal] = createSignal(false);

  const save = (next: ShellPrefs) => {
    setPrefsSignal(next);
    try {
      storage()?.setItem(PREFS_KEY, JSON.stringify(next));
    } catch {
      // storage full / blocked: the layout just isn't remembered
    }
  };
  const patch = (part: Partial<ShellPrefs>) => save({ ...prefs(), ...part });

  const onResize = () => {
    setViewport(vw());
    setDesktop(compute());
    setAuto((a) => autoCollapse(prefs(), a, vw()));
  };
  if (hasWindow) window.addEventListener('resize', onResize);
  const mq = hasMatchMedia ? window.matchMedia(`(min-width: ${DESKTOP_MIN_WIDTH}px)`) : undefined;
  const onMq = () => setDesktop(compute());
  mq?.addEventListener?.('change', onMq);

  const sidebarOpen = () => prefs().sidebarOpen && !auto().sidebar;
  const sideOpen = () => prefs().sideOpen && !auto().side;
  const sidebarWidth = () => clampSidebar(prefs().sidebarW, viewport());
  const mode = (): SideMode => sideMode(viewport());

  const api = {
    /** Desktop shell on (sidebar, panes, palette)? */
    desktop,
    viewport,
    prefs,
    sidebarOpen,
    sidebarWidth,
    sideOpen,
    sideWidth: () => clampSide(prefs().sideW, viewport(), sidebarOpen() ? sidebarWidth() : 0),
    sideMode: mode,
    bottomOpen: () => prefs().bottomOpen,
    bottomHeight: () => prefs().bottomH,
    fullView: () => fullView() && sideOpen() && desktop(),
    toggleSidebar(open = !sidebarOpen()) {
      setAuto((a) => ({ ...a, sidebar: false }));
      patch({ sidebarOpen: open });
    },
    setSidebarWidth(width: number) {
      patch({ sidebarW: clampSidebar(width, viewport()) });
    },
    toggleSide(open = !sideOpen()) {
      setAuto((a) => ({ ...a, side: false }));
      if (!open) setFullViewSignal(false);
      patch({ sideOpen: open });
    },
    setSideWidth(width: number) {
      patch({ sideW: clampSide(width, viewport(), sidebarOpen() ? sidebarWidth() : 0) });
    },
    toggleBottom(open = !prefs().bottomOpen) {
      patch({ bottomOpen: open });
    },
    setBottomHeight(height: number, frame: number) {
      patch({ bottomH: clampBottom(height, frame) });
    },
    /** Full view (Z-6): the side pane covers the conversation frame; opening it opens the pane. */
    toggleFullView(on = !api.fullView()) {
      if (on) api.toggleSide(true);
      setFullViewSignal(on);
    },
    dispose() {
      if (hasWindow) window.removeEventListener('resize', onResize);
      mq?.removeEventListener?.('change', onMq);
    },
  };
  return api;
}

export type Layout = ReturnType<typeof createLayout>;

let shared: Layout | undefined;
export function layout(): Layout {
  if (!shared) shared = createRoot(() => createLayout());
  return shared;
}
/** Test hook. */
export function resetLayout(): void {
  shared?.dispose();
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
