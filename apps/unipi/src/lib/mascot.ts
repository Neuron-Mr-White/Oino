/**
 * The floating Unicrab (UNI-200): pure logic behind the AssistiveTouch-style
 * button (components/FloatingMascot.tsx), the desktop mascot window and the
 * Android overlay feed — which items pop a chat bubble, what it says, and
 * where the button rests (snapped to a side, remembered).
 */
import type { NeedsYouItem } from './needsYou';
import type { HostInfo } from './schemas';
import { resolveHostId } from './notifyInit';

/** The text pi's ntfy sends when the user chose "minimal" detail: no summary in it. */
export const GENERIC_NTFY_BODY = 'Tap to open in UniPi';
export const BUBBLE_MAX = 140;
/** How long one bubble stays up (fade in + read + fade out). */
export const BUBBLE_MS = 6000;

/** The bubble line for an item: the summarized reply / question pi sent
 *  (the same `body` ntfy carries, or the chat_list waiting title), else a
 *  plain "… finished / needs you". */
export function bubbleText(item: Pick<NeedsYouItem, 'what' | 'title' | 'group' | 'hostName'>): string {
  let what = item.what?.trim();
  // pi's recap is "<session>: <summary>"; the bubble already heads with the session.
  const prefix = `${item.title}: `;
  if (what && item.title && what.toLowerCase().startsWith(prefix.toLowerCase())) what = what.slice(prefix.length).trim();
  const text = what && what !== GENERIC_NTFY_BODY ? what : item.group === 'finished' ? `${item.title} finished` : `${item.title} needs you`;
  const chars = Array.from(text.replace(/\s+/g, ' '));
  return chars.length > BUBBLE_MAX ? `${chars.slice(0, BUBBLE_MAX - 1).join('')}…` : chars.join('');
}

/** Items that should pop a bubble: new since the last look, unread, waiting or
 *  finished, oldest first (so they play in order). The first look only seeds
 *  (`seen` undefined → none), so opening the app doesn't replay history. */
export function freshBubbles(seen: ReadonlySet<string> | undefined, items: readonly NeedsYouItem[], isRead: (id: string) => boolean): NeedsYouItem[] {
  if (!seen) return [];
  return items
    .filter((i) => !seen.has(i.id) && !isRead(i.id) && (i.group === 'waiting' || i.group === 'finished'))
    .sort((a, b) => a.since - b.since);
}

/** The paired host an item belongs to: its id, or (ntfy items carry the PC's
 *  hostname) a name match. */
export function hostForItem(hosts: readonly HostInfo[], item: Pick<NeedsYouItem, 'hostId'>): HostInfo | undefined {
  const id = resolveHostId(hosts, item.hostId);
  return hosts.find((h) => h.id === id);
}

// ── Position: snapped to the left or right edge, remembered ───────────

export type Side = 'left' | 'right';
/** `y` is the button's top as a fraction of the free height (0 = top limit, 1 = bottom limit). */
export type MascotPos = { side: Side; y: number };
export type Bounds = { width: number; height: number; top: number; bottom: number; margin: number };

export const DEFAULT_POS: MascotPos = { side: 'right', y: 0 };
const POS_KEY = 'unipi.mascot.pos.v1';

/** Where a released drag lands: the nearer side, y clamped to [top, bottom]. */
export function snap(x: number, y: number, size: number, b: Bounds): MascotPos {
  const side: Side = x + size / 2 < b.width / 2 ? 'left' : 'right';
  const free = Math.max(1, b.height - b.top - b.bottom - size);
  const frac = (y - b.top) / free;
  return { side, y: Math.min(1, Math.max(0, frac)) };
}

/** A stored position → pixels (left, top) for the current viewport. */
export function place(pos: MascotPos, size: number, b: Bounds): { left: number; top: number } {
  const free = Math.max(0, b.height - b.top - b.bottom - size);
  return {
    left: pos.side === 'left' ? b.margin : b.width - size - b.margin,
    top: Math.round(b.top + Math.min(1, Math.max(0, pos.y)) * free),
  };
}

export function loadPos(): MascotPos {
  try {
    const raw = JSON.parse(localStorage.getItem(POS_KEY) ?? 'null') as Partial<MascotPos> | null;
    if (raw && (raw.side === 'left' || raw.side === 'right') && typeof raw.y === 'number' && Number.isFinite(raw.y))
      return { side: raw.side, y: Math.min(1, Math.max(0, raw.y)) };
  } catch {
    // corrupt → default
  }
  return DEFAULT_POS;
}

export function savePos(pos: MascotPos): void {
  try {
    localStorage.setItem(POS_KEY, JSON.stringify(pos));
  } catch {
    // storage unavailable: the position just isn't remembered
  }
}

/** A pointer that moved less than this is a tap / long-press, not a drag. */
export const DRAG_SLOP = 8;
export const LONG_PRESS_MS = 450;
/** No touch for this long → the idle button fades to half (unless something needs you). */
export const IDLE_FADE_MS = 3000;

// ── Prefs (device-local, like desktop notifications) ──────────────────

const FLOAT_KEY = 'unipi.mascot.floating';
const OVERLAY_KEY = 'unipi.mascot.overlay';
const WINDOW_KEY = 'unipi.mascot.window';

function readFlag(key: string, dflt: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    return v === null ? dflt : v === 'on';
  } catch {
    return dflt;
  }
}
function writeFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? 'on' : 'off');
  } catch {
    // ignore
  }
}

/** In-app floating crab on the phone (default on — it replaces the floating bell). */
export const floatingOn = () => readFlag(FLOAT_KEY, true);
export const setFloatingOn = (on: boolean) => writeFlag(FLOAT_KEY, on);
/** Android "over other apps" overlay (opt-in). */
export const overlayOn = () => readFlag(OVERLAY_KEY, false);
export const setOverlayOn = (on: boolean) => writeFlag(OVERLAY_KEY, on);
/** Desktop always-on-top mascot window (opt-in). */
export const mascotWindowOn = () => readFlag(WINDOW_KEY, false);
export const setMascotWindowOn = (on: boolean) => writeFlag(WINDOW_KEY, on);

/** UNI-255: the desktop mascot window's crab size (px wide): S / M / L. */
export const MASCOT_WINDOW_SIZES = [72, 96, 128] as const;
export type MascotWindowSize = (typeof MASCOT_WINDOW_SIZES)[number];
export const DEFAULT_MASCOT_WINDOW_SIZE: MascotWindowSize = 96;
const WINDOW_SIZE_KEY = 'unipi.mascot.windowSize';
export function mascotWindowSize(): MascotWindowSize {
  try {
    const n = Number(localStorage.getItem(WINDOW_SIZE_KEY));
    return (MASCOT_WINDOW_SIZES as readonly number[]).includes(n) ? (n as MascotWindowSize) : DEFAULT_MASCOT_WINDOW_SIZE;
  } catch {
    return DEFAULT_MASCOT_WINDOW_SIZE;
  }
}
export function setMascotWindowSize(size: MascotWindowSize): void {
  try {
    localStorage.setItem(WINDOW_SIZE_KEY, String(size));
  } catch {
    // ignore
  }
}

// ── What the overlay / mascot window is told ──────────────────────────

export type MascotFeed = {
  state: import('./unicrab').MascotState;
  /** Unread count for the badge. */
  count: number;
  /** A bubble to show now (once), or null. */
  bubble: { id: string; text: string; link: string } | null;
  /** UNI-255: crab width in the desktop mascot window (absent = default). */
  size?: number;
};

/** `unipi://chat?host=&pid=` for an item (the deep link the overlay / notification taps open). */
export function chatLink(item: Pick<NeedsYouItem, 'hostId' | 'pid'>): string {
  const q = new URLSearchParams();
  q.set('host', item.hostId);
  if (item.pid !== null) q.set('pid', String(item.pid));
  return `unipi://chat?${q.toString()}`;
}
