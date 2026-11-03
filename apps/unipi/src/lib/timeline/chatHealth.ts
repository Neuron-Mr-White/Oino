/**
 * chat-vanish self-heal: "the shown chat shows nothing while the host is live" as a pure
 * verdict, plus the bounded retry budget the timeline's watchdog spends on it.
 *
 * Owner 2026-11-03: "occasionally the chat fully disappears, and only a full app restart
 * brings it back." Every reason below is a state the timeline used to render as an empty
 * pane with no way back; the watchdog re-opens the stream / re-mounts the list instead and
 * logs why (`lib/appLog`), and the timeline always draws *something* (skeleton, error card).
 */
import type { ConnState } from '../session/types';

export type BlankReason =
  /** The view holds a handle the registry closed (LRU eviction, archive). */
  | 'handle_closed'
  /** The stream ended without an error and nothing reopens it. */
  | 'stream_closed'
  /** Reconnects gave up (2 min) — the host is back, the chat is not. */
  | 'stream_lost'
  /** The open was rejected and its own retries are spent. */
  | 'open_failed'
  /** Live, turns loaded, the virtual list renders no row (0-height scroller window). */
  | 'no_rows';

/** What the reader asked for ("Reload chat", the error card's Retry) — healed like a reason. */
export type HealCause = BlankReason | 'manual' | 'crash';

export type ChatHealthInput = {
  hostOnline: boolean;
  handleClosed: boolean;
  conn: ConnState;
  connErrorCode?: string;
  takenOver: boolean;
  archived: boolean;
  /** Settled turns in the list (the live tail is outside it). */
  settled: number;
  /** Rows the virtualizer rendered. */
  rows: number;
};

/** Why this chat would stay blank, or `undefined` when it is fine / in progress / not ours to fix. */
export function blankReason(i: ChatHealthInput): BlankReason | undefined {
  // Offline hosts and taken-over / archived chats have their own banners; a missing
  // session (E_NOT_FOUND) is routed away by the workspace — never reopened in a loop.
  if (!i.hostOnline || i.takenOver || i.archived) return undefined;
  if (i.handleClosed) return 'handle_closed';
  switch (i.conn) {
    case 'connecting':
    case 'reconnecting':
      return undefined;
    case 'lost':
      return 'stream_lost';
    case 'closed':
      if (i.connErrorCode === 'E_NOT_FOUND') return undefined;
      return i.connErrorCode ? 'open_failed' : 'stream_closed';
    case 'live':
      return i.settled > 0 && i.rows === 0 ? 'no_rows' : undefined;
    default: {
      const never: never = i.conn;
      return never;
    }
  }
}

/** How long a blank state must last before the watchdog acts (a load gets its 2 s). */
export const HEAL_AFTER_MS = 2_000;
/** Heals per chat per minute; past it the error card's Retry is the way (no reopen loop). */
export const HEALS_PER_MINUTE = 4;

/** A per-chat budget: `allow()` at most `max` times in any `windowMs`. Bounded (≤ max stamps). */
export function healBudget(max = HEALS_PER_MINUTE, windowMs = 60_000, now: () => number = Date.now) {
  const stamps: number[] = [];
  return {
    allow(): boolean {
      const t = now();
      while (stamps.length && t - stamps[0]! >= windowMs) stamps.shift();
      if (stamps.length >= max) return false;
      stamps.push(t);
      return true;
    },
  };
}
