/**
 * The open chat (one at a time; the layer over the app shows it). Opening
 * the same session again resumes the live connection. WP-A1 placeholder
 * (moved from lib/chat/open.ts): WP-A3 replaces it with routes
 * (`/s/:host/:session`) and the session store LRU.
 */
import { createRoot, createSignal } from 'solid-js';
import { createChat, type Chat } from '../chat/store';
import { requestBoardJump } from '../board/portal';
import type { ChatTarget } from './list';

export type OpenChat = { target: ChatTarget; pcName: string; chat: Chat };

const [current, setCurrent] = createSignal<OpenChat | null>(null);
const [visible, setVisible] = createSignal(false);

export const openChatState = current;
export const chatVisible = visible;

export function openChat(target: ChatTarget, pcName: string): void {
  const cur = current();
  if (cur && cur.target.hostId === target.hostId && cur.target.session === target.session) {
    setVisible(true);
    return;
  }
  cur?.chat.close();
  const chat = createRoot(() => createChat(target.hostId, target.session));
  setCurrent({ target, pcName, chat });
  setVisible(true);
}

/** Fills in the open chat's list labels (title, workspace…) once they're known — a chat opened
 * from a notification deep link starts with placeholders (UNI-161). Same session only; the live
 * connection is kept. */
export function updateOpenTarget(target: ChatTarget): void {
  const cur = current();
  if (!cur || cur.target.hostId !== target.hostId || cur.target.session !== target.session) return;
  setCurrent({ ...cur, target });
}

/** Back to the list; the connection stays (instant resume). */
export function hideChat(): void {
  setVisible(false);
}

/** Shows the open chat again (board → chat portal, UNI-181); false when none is open. */
export function showChat(): boolean {
  if (!current()) return false;
  setVisible(true);
  return true;
}

/** Hang up. */
export function closeChat(): void {
  current()?.chat.close();
  setCurrent(null);
  setVisible(false);
}

/** Set when a screen (board, files) left a chat; leaving that screen returns to it. */
let returnToChat = false;
export function markReturnToChat(): void {
  returnToChat = true;
}
/** Whether a screen left a chat with the return mark still armed. */
export function hasMarkedReturnToChat(): boolean {
  return returnToChat;
}
/** Called when that screen is left; reopens the chat it came from (once). */
export function resumeChatAfterTerminal(): boolean {
  if (!returnToChat || !current()) return false;
  returnToChat = false;
  setVisible(true);
  return true;
}
export function clearReturnToChat(): void {
  returnToChat = false;
}

/**
 * Desktop shell only (UNI-312): decides what a sidebar tab switch does to
 * the chat layer. Pure (keyed only off the screen the route *was* on and
 * the screen it just became) so the transition is unit-testable without
 * mounting the shell — see lib/session/open.test.ts.
 *
 *   any screen → 'chat'   an open chat comes back (sidebar "Chat" = show
 *                         again, draft/scroll untouched — same OpenChat
 *                         object, just a visibility flip); with no chat
 *                         open there's nothing to show, so `undefined`
 *                         (the Chat screen's own list renders as usual).
 *   'chat' → any screen   hidden, so the new screen's pane shows instead
 *                         of the chat layer sitting on top of it.
 *   same screen twice,
 *   or no previous screen  `undefined` (no override) — covers mounting,
 *                         and keeps this out of the way of the chat's own
 *                         hideChat()/showChat() calls (Back to the list,
 *                         the board/file-ref portals…), which don't
 *                         change the *screen* and must not be fought here.
 *
 * Returns `undefined` for "leave `chatVisible` alone"; a `boolean` is the
 * new value the caller should apply.
 */
export function chatVisibilityOnScreenChange(
  prevScreen: string | undefined,
  nextScreen: string,
  hasOpenChat: boolean,
): boolean | undefined {
  if (prevScreen === undefined || prevScreen === nextScreen) return undefined;
  if (nextScreen === 'chat') return hasOpenChat ? true : undefined;
  if (prevScreen === 'chat') return false;
  return undefined;
}

/** Applies {@link chatVisibilityOnScreenChange} (desktop shell only —
 * call from an effect that tracks the screen key, e.g. DesktopChrome). */
export function syncChatVisibleOnScreenChange(prevScreen: string | undefined, nextScreen: string): void {
  const next = chatVisibilityOnScreenChange(prevScreen, nextScreen, current() !== null);
  if (next !== undefined) setVisible(next);
}

// ── Chat → board portal (UNI-181) ────────────────────────────────────────
// Hide the chat, mark the return, hand the Board screen a jump and switch to
// its tab. The Board returns to this chat when the task pane closes.

function goBoard(): void {
  hideChat();
  markReturnToChat();
  // Hash route switch (works outside a router-scoped component, e.g. MdBlock).
  if (typeof location !== 'undefined') location.hash = '#/board';
}

/** Opens a task's detail pane on the board. */
export function openTaskFromChat(hostId: string, taskId: string, cwd?: string): void {
  requestBoardJump({ hostId, taskId, cwd, fromChat: true });
  goBoard();
}

/** Opens the board's New task sheet (project picked from the chat's cwd). */
export function addTaskFromChat(hostId: string, cwd?: string): void {
  requestBoardJump({ hostId, newTask: true, cwd, fromChat: true });
  goBoard();
}

/** Just the board, no pane (header menu "Kanboard"). */
export function openBoardFromChat(): void {
  hideChat();
  if (typeof location !== 'undefined') location.hash = '#/board';
}
