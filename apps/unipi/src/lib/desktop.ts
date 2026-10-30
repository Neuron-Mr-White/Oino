/**
 * Desktop-first input + native bits (UNI-117 part 2). Pure decision
 * helpers (unit-tested) for the keyboard on the desktop shell, plus the
 * thin bridge to the desktop core's commands (tray status, OS
 * notifications, window title). Everything here is only *called* when
 * the desktop shell is on (`desktopShell()`) or the platform has the
 * capability (`can('tray')` …); the phone never reaches it.
 *
 * The shortcut list itself moved to `lib/shortcuts/` (WP-A18, Z-94).
 */
import { can, platform } from './platform';

type Keyish = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'> & { isComposing?: boolean; keyCode?: number };

const mod = (e: Keyish) => e.ctrlKey || e.metaKey;

// ── Chat composer ──────────────────────────────────────────────────────

export type ComposerKeyAction =
  /** Enter, idle: send now. */
  | 'send'
  /** Enter while pi works: send with this chat's last Steer/Queue choice (Steer the first time — DESIGN §3.4, like Enter in pi's TUI). */
  | 'send-default'
  /** Ctrl/⌘+Enter while pi works: open the Steer / Queue choice. */
  | 'choose'
  /** Esc while pi works (nothing else to close): stop the run. */
  | 'stop'
  | null;

/**
 * Desktop composer keys (DESIGN §5.8): Enter sends, Shift+Enter is a new
 * line, Ctrl/⌘+Enter offers Steer or Queue while pi works, Esc stops a
 * run. Menus (/ and @ suggestions, the choice bar) are handled by the
 * composer before this.
 */
export function composerKey(e: Keyish, s: { running: boolean; empty: boolean }): ComposerKeyAction {
  if (e.isComposing || e.keyCode === 229) return null;
  if (e.key === 'Escape') return s.running && !e.shiftKey && !mod(e) ? 'stop' : null;
  if (e.key !== 'Enter' || e.shiftKey || e.altKey) return null;
  if (s.empty) return null;
  if (mod(e)) return s.running ? 'choose' : 'send';
  return s.running ? 'send-default' : 'send';
}

/** Keys inside the open Steer/Queue bar: ←/→/Tab switch, S/Q pick, Enter picks the highlighted one. */
export function choiceKey(e: Keyish, current: 'steer' | 'after'): { pick?: 'steer' | 'after'; highlight?: 'steer' | 'after'; close?: boolean } | null {
  if (e.key === 'Escape') return { close: true };
  if (e.key === 'Enter') return { pick: current };
  if (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'Tab') return { highlight: current === 'steer' ? 'after' : 'steer' };
  const k = e.key.toLowerCase();
  if (!mod(e) && !e.altKey && k === 's') return { pick: 'steer' };
  if (!mod(e) && !e.altKey && k === 'q') return { pick: 'after' };
  return null;
}

/** Alt+↑/↓: the chat before/after `current` in the list order (wraps; skips nothing — callers pass only openable chats). */
export function adjacentIndex(count: number, current: number, delta: 1 | -1): number | undefined {
  if (count <= 0) return undefined;
  if (current < 0) return delta > 0 ? 0 : count - 1;
  return (current + delta + count) % count;
}

/** Files dropped on a window: split into images (shrunk + sent inline) and other files (uploaded as attachments). */
export function splitDropped(files: readonly File[]): { images: File[]; others: File[] } {
  const images: File[] = [];
  const others: File[] = [];
  for (const f of files) (f.type.startsWith('image/') && !/svg/.test(f.type) ? images : others).push(f);
  return { images, others };
}

/** Does this drag carry files from the OS (not text, not an in-page drag)? */
export function dragHasFiles(dt: Pick<DataTransfer, 'types'> | null | undefined): boolean {
  return !!dt && Array.from(dt.types ?? []).includes('Files');
}

// ── Terminal ───────────────────────────────────────────────────────────

export type TermKeyAction =
  | { kind: 'copy' }
  | { kind: 'paste' }
  | { kind: 'font'; delta: 1 | -1 | 0 }
  /** Not for the terminal: let the app's own shortcut handle it (Ctrl+1…9 screens, Ctrl+Shift+K palette, ⌘ combos on a Mac). */
  | { kind: 'app' }
  | null;

/**
 * A physical keyboard on the desktop terminal (DESIGN §5.8). xterm gets
 * every key itself (Ctrl/Alt combos, arrows, F-keys, Esc…) except:
 * Ctrl+Shift+C / Ctrl+Shift+V copy/paste (Linux terminal convention;
 * ⌘C/⌘V on a Mac), Ctrl+= / Ctrl+- / Ctrl+0 font size, and the app's
 * own Ctrl+1…9 and Ctrl+Shift+K (plain Ctrl+K stays the shell's).
 */
export function termKey(e: Keyish, mac = false): TermKeyAction {
  const k = e.key;
  const lower = k.toLowerCase();
  if (mac && e.metaKey && !e.ctrlKey && !e.altKey) {
    if (lower === 'c') return { kind: 'copy' };
    if (lower === 'v') return { kind: 'paste' };
  }
  if (e.ctrlKey && e.shiftKey && !e.altKey) {
    if (lower === 'c') return { kind: 'copy' };
    if (lower === 'v') return { kind: 'paste' };
    if (lower === 'k') return { kind: 'app' };
  }
  if (mod(e) && !e.altKey) {
    if (k === '=' || k === '+') return { kind: 'font', delta: 1 };
    if (k === '-' || k === '_') return { kind: 'font', delta: -1 };
    if (k === '0') return { kind: 'font', delta: 0 };
    if (/^[1-9]$/.test(k) && !e.shiftKey) return { kind: 'app' };
  }
  if (mac && e.metaKey) return { kind: 'app' };
  return null;
}

export const TERM_FONT_MIN = 8;
export const TERM_FONT_MAX = 32;
export const TERM_FONT_DEFAULT = 14;
export function nextFontSize(current: number, delta: 1 | -1 | 0): number {
  if (delta === 0) return TERM_FONT_DEFAULT;
  return Math.min(TERM_FONT_MAX, Math.max(TERM_FONT_MIN, current + delta));
}

// ── Files ──────────────────────────────────────────────────────────────

export type FilesKeyAction = { kind: 'move'; to: number } | { kind: 'open' } | { kind: 'up' } | { kind: 'rename' } | { kind: 'delete' } | null;

/** Keyboard in the desktop Files list: ↑/↓/Home/End move, Enter opens, Backspace goes up, F2 renames, Del deletes (asks). */
export function filesKey(e: Keyish, index: number, count: number): FilesKeyAction {
  if (mod(e) || e.altKey) return null;
  switch (e.key) {
    case 'ArrowDown':
      return count ? { kind: 'move', to: Math.min(count - 1, index < 0 ? 0 : index + 1) } : null;
    case 'ArrowUp':
      return count ? { kind: 'move', to: Math.max(0, index < 0 ? 0 : index - 1) } : null;
    case 'Home':
      return count ? { kind: 'move', to: 0 } : null;
    case 'End':
      return count ? { kind: 'move', to: count - 1 } : null;
    case 'Enter':
      return index >= 0 ? { kind: 'open' } : null;
    case 'Backspace':
      return { kind: 'up' };
    case 'F2':
      return index >= 0 ? { kind: 'rename' } : null;
    case 'Delete':
      return index >= 0 ? { kind: 'delete' } : null;
  }
  return null;
}

/** Is the key event aimed at a text field (input/textarea/contenteditable)? Global shortcuts skip those. */
export function inTextField(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || typeof el.closest !== 'function') return false;
  return !!el.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"], .cm-editor, .xterm');
}

// ── Native (desktop app) ───────────────────────────────────────────────

/** Window title: "<what> — UniPi". */
export function windowTitle(parts: Array<string | undefined>): string {
  const clean = parts.map((p) => p?.trim()).filter((p): p is string => !!p);
  return clean.length ? `${clean.join(' — ')} — UniPi` : 'UniPi';
}

async function invokeDesktop(cmd: string, args: Record<string, unknown>): Promise<boolean> {
  if (platform().mode !== 'tauri-desktop') return false;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke(cmd, args);
    return true;
  } catch (error) {
    console.warn(`[desktop] ${cmd} failed`, error);
    return false;
  }
}

let lastTitle = '';
/** Sets the document + native window title (desktop app; web sets document.title only). */
export function setWindowTitle(title: string): void {
  if (!can('windowTitle')) return;
  if (typeof document !== 'undefined') document.title = title;
  if (title === lastTitle) return;
  lastTitle = title;
  void invokeDesktop('desktop_set_title', { title });
}

export function nativeNotify(title: string, body: string): Promise<boolean> {
  if (!can('nativeNotifications')) return Promise.resolve(false);
  return invokeDesktop('desktop_notify', { title, body });
}

let lastStatus = '';
export function setTrayStatus(needs: number, running: number): void {
  if (!can('tray')) return;
  const key = `${needs}/${running}`;
  if (key === lastStatus) return;
  lastStatus = key;
  void invokeDesktop('desktop_status', { needs, running });
}

const NOTIFY_KEY = 'unipi.desktop.notify';
export function desktopNotifyOn(): boolean {
  try {
    return localStorage.getItem(NOTIFY_KEY) !== 'off';
  } catch {
    return true;
  }
}
export function setDesktopNotifyOn(on: boolean): void {
  try {
    localStorage.setItem(NOTIFY_KEY, on ? 'on' : 'off');
  } catch {
    // not remembered
  }
}
