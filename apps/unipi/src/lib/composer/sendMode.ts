/**
 * Follow-up mode (WP-A11, Z-52 — ruling: ZCode's global default + modifier):
 * one **global** setting decides what Enter does while a run is going —
 *
 *   queue  → the message waits until the run ends (`follow_up`)
 *   steer  → it is delivered mid-run at the next tool boundary (`steer`)
 *
 * Enter always does the default, Cmd/Ctrl+Enter the opposite once. The
 * chevron menu (`components/composer/SendButton.tsx`) switches the default
 * and keeps unipi's "Send now (stops run)". Replaces per-chat `sendChoice`
 * and the old S/Q inline choice (deleted with `lib/chat/sendChoice.ts`).
 */
import { createSignal } from 'solid-js';

export type FollowUpMode = 'queue' | 'steer';

const STORAGE_KEY = 'unipi:composer:followUpMode';

let storage: Pick<Storage, 'getItem' | 'setItem'> | undefined = typeof localStorage === 'undefined' ? undefined : localStorage;

/** Tests: swap persistence (or drop it with `undefined`). */
export function setFollowUpModeStorage(next: Pick<Storage, 'getItem' | 'setItem'> | undefined): void {
  storage = next;
}

function read(): FollowUpMode {
  try {
    return storage?.getItem(STORAGE_KEY) === 'steer' ? 'steer' : 'queue';
  } catch {
    return 'queue';
  }
}

const [mode, setMode] = createSignal<FollowUpMode>(read());

/** The global default (`queue` unless set otherwise). Reactive. */
export function followUpMode(): FollowUpMode {
  return mode();
}

/** Sets the global default (persists; survives restarts). */
export function setFollowUpMode(next: FollowUpMode): void {
  setMode(next);
  try {
    storage?.setItem(STORAGE_KEY, next);
  } catch {
    // storage full/unavailable: just not remembered next launch.
  }
}

export function oppositeMode(of: FollowUpMode): FollowUpMode {
  return of === 'queue' ? 'steer' : 'queue';
}

/** A keyboard event's modifier (Cmd on macOS, Ctrl elsewhere). */
export function hasSendModifier(ev: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey'>): boolean {
  return ev.metaKey || ev.ctrlKey;
}

/**
 * Which follow-up mode a plain Enter / Cmd/Ctrl+Enter expresses: the default,
 * or the opposite when the modifier is held (Z-52).
 */
export function enterMode(ev: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey'>): FollowUpMode {
  return hasSendModifier(ev) ? oppositeMode(mode()) : mode();
}

/** The session command mode that mode means right now: while idle every send is a prompt. */
export function outboxMode(m: FollowUpMode, running: boolean): 'prompt' | 'steer' | 'followUp' {
  if (!running) return 'prompt';
  return m === 'steer' ? 'steer' : 'followUp';
}

/** Short label for the menu / chips. */
export function modeLabel(m: FollowUpMode): string {
  return m === 'steer' ? 'Steer' : 'Queue';
}

/**
 * The send mode for one Enter keypress against a session's run state —
 * the single entry point composers use:
 *
 *   idle:  Enter and Cmd/Ctrl+Enter both send a prompt
 *   running: Enter → the default (queue ⇒ `followUp`, steer ⇒ `steer`),
 *            Cmd/Ctrl+Enter → the opposite
 */
export function sendModeForEnter(ev: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey'>, running: boolean): 'prompt' | 'steer' | 'followUp' {
  return outboxMode(enterMode(ev), running);
}
