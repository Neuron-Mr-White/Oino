/**
 * Platform layer (UNI-117): which runtime the UI is in and what it can do.
 * Every feature asks `can(cap)` (or wraps itself in `<Requires cap=…>`)
 * instead of sniffing `__TAURI_INTERNALS__` itself.
 *
 * Modes:
 * - `tauri-mobile`  — the Android (or iOS) app: native core, iroh, plugins
 *                     and the MainActivity JS bridges (haptics, notify, update).
 * - `tauri-desktop` — the Linux/Windows/macOS app: native core and iroh, no
 *                     phone-only plugins (camera, haptics, APK updater).
 * - `web`           — a plain browser: `vite dev`, or the web build served by
 *                     the kanboard daemon. No native core, so no iroh: only the
 *                     Board works (same-origin kanboard JSON API).
 *
 * Detection is synchronous: Tauri injects `__TAURI_INTERNALS__` and the os
 * plugin injects `__TAURI_OS_PLUGIN_INTERNALS__.platform` before any app code
 * runs. Nothing is cached, so tests can flip globals (or use
 * `setPlatformOverride`).
 *
 * See DESIGN.md "Platforms" for the capability matrix and the rule for new
 * features (declare the caps you need + a fallback).
 */

export type PlatformMode = 'tauri-mobile' | 'tauri-desktop' | 'web';
export type Os = 'android' | 'ios' | 'linux' | 'windows' | 'macos' | 'unknown';

export type Platform = {
  mode: PlatformMode;
  os: Os;
  /** Has the native core (invoke, plugins, iroh). */
  tauri: boolean;
};

/**
 * Capabilities. Add a new one here, give it a row in `MATRIX` and in
 * DESIGN.md "Platforms", and a label in `CAP_LABEL`.
 */
export type Capability =
  /** Paired PCs over iroh (hosts list, pairing, connection status). */
  | 'hosts'
  /** Live terminal streams (term_open). */
  | 'terminal'
  /** Chat sessions (host session streams). */
  | 'chat'
  /** Remote filesystem browser/editor (fs_*). */
  | 'files'
  /** Blob upload/download (attachments, previews of PC files). */
  | 'blob'
  /** Kanban board (daemon over iroh, or same-origin in web mode). */
  | 'board'
  /** Port forwarding / Preview (loopback listener in the core). */
  | 'portForward'
  /** Stream the PC's desktop with control (WebCodecs + iroh, UNI-216). */
  | 'desktopStream'
  /** Push + local notifications (Android notify bridge). */
  | 'notifications'
  /** Haptic feedback. */
  | 'haptics'
  /** `unipi://` deep links (pair, chat). */
  | 'deepLinks'
  /** In-app updater (APK download + system installer). */
  | 'updater'
  /** Camera (QR pairing). */
  | 'camera'
  /** Drag files from the OS into the window. */
  | 'fileDragDrop'
  /** Clipboard read/write (web: navigator.clipboard, may prompt). */
  | 'clipboard'
  /** Native app menus / tray. */
  | 'nativeMenus'
  /** More than one app window. */
  | 'multiWindow'
  /** Hardware Back button. */
  | 'backButton'
  /** Window title follows the current screen. */
  | 'windowTitle'
  /** Native OS notifications from the desktop app (tauri-plugin-notification). */
  | 'nativeNotifications'
  /** System tray icon with show/hide + status (desktop app). */
  | 'tray'
  /** Window size/position remembered across launches (tauri-plugin-window-state). */
  | 'windowState'
  /** Switch this PC's own sharing (forwarding/browser/desktop/push) through its unipi-host CLI (UNI-248). */
  | 'localSharing'
  /** Desktop app self-update (signed, prompted; UNI-263). */
  | 'desktopUpdater';

/** Where a capability is available, by mode. */
const MATRIX: Record<Capability, Record<PlatformMode, boolean>> = {
  hosts: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  terminal: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  chat: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  files: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  blob: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  board: { 'tauri-mobile': true, 'tauri-desktop': true, web: true },
  portForward: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  desktopStream: { 'tauri-mobile': true, 'tauri-desktop': true, web: false },
  notifications: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  haptics: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  deepLinks: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  updater: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  camera: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  fileDragDrop: { 'tauri-mobile': false, 'tauri-desktop': true, web: true },
  clipboard: { 'tauri-mobile': true, 'tauri-desktop': true, web: true },
  nativeMenus: { 'tauri-mobile': false, 'tauri-desktop': false, web: false },
  multiWindow: { 'tauri-mobile': false, 'tauri-desktop': false, web: false },
  backButton: { 'tauri-mobile': true, 'tauri-desktop': false, web: false },
  windowTitle: { 'tauri-mobile': false, 'tauri-desktop': true, web: true },
  nativeNotifications: { 'tauri-mobile': false, 'tauri-desktop': true, web: false },
  tray: { 'tauri-mobile': false, 'tauri-desktop': true, web: false },
  windowState: { 'tauri-mobile': false, 'tauri-desktop': true, web: false },
  localSharing: { 'tauri-mobile': false, 'tauri-desktop': true, web: false },
  desktopUpdater: { 'tauri-mobile': false, 'tauri-desktop': true, web: false },
};

/** Human name of a capability, for notices ("Chat isn't available…"). */
export const CAP_LABEL: Record<Capability, string> = {
  hosts: 'Paired PCs',
  terminal: 'Terminals',
  chat: 'Chat',
  files: 'Files',
  blob: 'File transfer',
  board: 'The board',
  portForward: 'Preview (port forwarding)',
  desktopStream: 'Desktop streaming',
  notifications: 'Notifications',
  haptics: 'Haptics',
  deepLinks: 'unipi:// links',
  updater: 'In-app updates',
  camera: 'QR scanning',
  fileDragDrop: 'Drag and drop',
  clipboard: 'Clipboard',
  nativeMenus: 'App menus',
  multiWindow: 'Multiple windows',
  backButton: 'Back button',
  windowTitle: 'Window title',
  nativeNotifications: 'Desktop notifications',
  tray: 'Tray icon',
  windowState: 'Remembered window size',
  localSharing: "This PC's sharing switches",
  desktopUpdater: 'Desktop app updates',
};

export const CAPABILITIES = Object.keys(MATRIX) as Capability[];

type TauriWindow = {
  __TAURI_INTERNALS__?: unknown;
  __TAURI_OS_PLUGIN_INTERNALS__?: { platform?: string };
  /** Dev-only design-capture mocks (the docs capture scripts + their vite configs). */
  __UNIPI_MOCK__?: unknown;
  /** Capture harness mock host (vite.config.capture.ts, src/dev/mock/boot.ts). */
  __UNIPI_HOST__?: unknown;
  __UNIPI_MOCK_PLATFORM__?: PlatformMode;
};

function osFrom(name: string | undefined, userAgent: string): Os {
  switch (name) {
    case 'android':
    case 'ios':
    case 'linux':
    case 'windows':
    case 'macos':
      return name;
  }
  if (/Android/i.test(userAgent)) return 'android';
  if (/iPhone|iPad|iPod/i.test(userAgent)) return 'ios';
  if (/Windows/i.test(userAgent)) return 'windows';
  if (/Mac OS X|Macintosh/i.test(userAgent)) return 'macos';
  if (/Linux|X11/i.test(userAgent)) return 'linux';
  return 'unknown';
}

/** Pure detection from a window-like object (tests pass a fake). */
export function detectPlatform(win: unknown = typeof window !== 'undefined' ? window : undefined, userAgent?: string): Platform {
  const w = (win ?? {}) as TauriWindow;
  const ua = userAgent ?? (typeof navigator !== 'undefined' ? navigator.userAgent : '');
  const tauri = '__TAURI_INTERNALS__' in w;
  const os = osFrom(w.__TAURI_OS_PLUGIN_INTERNALS__?.platform, ua);
  if (!tauri) {
    // Design captures mock the native side in a plain browser (IPC through
    // `__UNIPI_MOCK__`, or the whole host through `__UNIPI_HOST__`): they render the
    // phone app, so keep its capabilities. `tauri` stays false, so the
    // transports still pick their browser/dev paths exactly as before.
    if (w.__UNIPI_MOCK__ || w.__UNIPI_HOST__) return { mode: w.__UNIPI_MOCK_PLATFORM__ ?? 'tauri-mobile', os, tauri: false };
    return { mode: 'web', os, tauri: false };
  }
  // Inside Tauri an unknown OS is treated as desktop unless the UA says phone.
  const mobile = os === 'android' || os === 'ios';
  return { mode: mobile ? 'tauri-mobile' : 'tauri-desktop', os, tauri: true };
}

let override: Platform | undefined;
/** Tests / captures force a platform; `undefined` restores detection. */
export function setPlatformOverride(next: Platform | PlatformMode | undefined): void {
  if (typeof next === 'string') {
    override = { mode: next, os: next === 'tauri-mobile' ? 'android' : next === 'tauri-desktop' ? 'linux' : 'unknown', tauri: next !== 'web' };
  } else override = next;
}

export function platform(): Platform {
  return override ?? detectPlatform();
}

/** Has the native core (Tauri invoke + plugins). */
export function isTauri(): boolean {
  return platform().tauri;
}
export function isWeb(): boolean {
  return platform().mode === 'web';
}
export function isTauriMobile(): boolean {
  return platform().mode === 'tauri-mobile';
}
export function isTauriDesktop(): boolean {
  return platform().mode === 'tauri-desktop';
}

/** Is `cap` available in the current (or given) platform? */
export function can(cap: Capability, p: Platform = platform()): boolean {
  return MATRIX[cap][p.mode];
}

/** Every capability with its availability, for Settings → About / debugging. */
export function capabilities(p: Platform = platform()): Record<Capability, boolean> {
  return Object.fromEntries(CAPABILITIES.map((cap) => [cap, can(cap, p)])) as Record<Capability, boolean>;
}

export type UnavailableKind = 'needs-app' | 'desktop-only' | 'phone-only' | 'not-yet';

export type Unavailable = { kind: UnavailableKind; title: string; body: string };

/**
 * Why `cap` isn't available here, as the standard notice text. `undefined`
 * when it is available.
 * - web, works in the app → "Not available in the web version — open the UniPi app"
 * - app, desktop-only      → "Desktop only"
 * - app, phone-only        → "Phone only"
 * - nowhere yet            → "Not available yet"
 */
export function unavailable(cap: Capability, p: Platform = platform()): Unavailable | undefined {
  if (can(cap, p)) return undefined;
  const name = CAP_LABEL[cap];
  const onMobile = MATRIX[cap]['tauri-mobile'];
  const onDesktop = MATRIX[cap]['tauri-desktop'];
  if (p.mode === 'web' && (onMobile || onDesktop)) {
    return {
      kind: 'needs-app',
      title: 'Not available in the web version',
      body: `${name} — open the Unipi app on your phone or computer to use this. The web version can't reach your PC directly.`,
    };
  }
  if (onDesktop && !onMobile) return { kind: 'desktop-only', title: 'Desktop only', body: `${name} works in Unipi on your computer.` };
  if (onMobile && !onDesktop) return { kind: 'phone-only', title: 'Phone only', body: `${name} works in Unipi on your phone.` };
  return { kind: 'not-yet', title: 'Not available yet', body: `${name} isn't available on this platform yet.` };
}
