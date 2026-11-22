/**
 * v2 chat route navigation + cross-screen return-route portal (P0-07):
 * routes conversations through `#/s/<host>/<session>` and new drafts through
 * `#/new/<host>?cwd=`, keeping a reactive return-route so opening Board, Files
 * or Editor from a chat returns to that exact chat on close.
 */
import { createSignal } from 'solid-js';
import { requestBoardJump } from '../board/portal';
import { desktopShell } from '../layout';
import { appNavigate } from '../nav';
import { bareTarget, type ChatTarget } from './list';
import { sessions } from './store';

export type ActiveChatRoute =
  | { kind: 'session'; hostId: string; sessionId: string; mentionKey: string; route: string }
  | { kind: 'draft'; hostId: string; cwd?: string; mentionKey: string; route: string };

export type OpenChat = { target: ChatTarget; pcName: string };

const [returnRoute, setReturnRoute] = createSignal<string | null>(null);
const [routeTick, setRouteTick] = createSignal(0);

/** `/s/<host>/<session>` route path. */
export function openChatHref(hostId: string, sessionId: string): string {
  return `/s/${encodeURIComponent(hostId)}/${encodeURIComponent(sessionId)}`;
}

/** `/new/<host>?cwd=` route path. */
export function newChatHref(hostId: string, cwd?: string): string {
  const trimmed = cwd?.trim();
  return `/new/${encodeURIComponent(hostId)}${trimmed ? `?cwd=${encodeURIComponent(trimmed)}` : ''}`;
}

/** Parses a hash or pathname (`/s/:host/:session` or `/new/:host?cwd=`) into an `ActiveChatRoute`. */
export function parseChatRoute(raw: string | undefined): ActiveChatRoute | undefined {
  if (!raw) return undefined;
  const path = raw.startsWith('#') ? raw.slice(1) : raw;
  const sess = /^\/s\/([^/?#]+)\/([^/?#]+)/.exec(path);
  if (sess?.[1] && sess[2]) {
    const hostId = decodeURIComponent(sess[1]);
    const sessionId = decodeURIComponent(sess[2]);
    return {
      kind: 'session',
      hostId,
      sessionId,
      mentionKey: `${hostId}:${sessionId}`,
      route: openChatHref(hostId, sessionId),
    };
  }
  const draft = /^\/new\/([^/?#]+)(?:\?([^#]*))?/.exec(path);
  if (draft?.[1]) {
    const hostId = decodeURIComponent(draft[1]);
    const params = draft[2] ? new URLSearchParams(draft[2]) : undefined;
    const cwd = params?.get('cwd') || undefined;
    return {
      kind: 'draft',
      hostId,
      ...(cwd ? { cwd } : {}),
      mentionKey: `draft:${hostId}`,
      route: newChatHref(hostId, cwd),
    };
  }
  return undefined;
}

/** Current route if the router is showing a conversation or new-chat draft. */
export function currentChatRoute(): string | undefined {
  routeTick();
  if (typeof location === 'undefined') return undefined;
  return parseChatRoute(location.hash)?.route;
}

/**
 * Resolves the active or most-recently-used v2 chat target (optionally scoped
 * to `hostId`): current route → armed return-route → session store MRU.
 */
export function activeChatTargetFor(hostId?: string): ActiveChatRoute | undefined {
  routeTick();
  const matchesHost = (candidate: string) => !hostId || candidate === hostId;
  if (typeof location !== 'undefined') {
    const cur = parseChatRoute(location.hash);
    if (cur && matchesHost(cur.hostId)) return cur;
  }
  const back = parseChatRoute(returnRoute() ?? undefined);
  if (back && matchesHost(back.hostId)) return back;
  const keys = sessions().keys();
  for (let i = keys.length - 1; i >= 0; i -= 1) {
    const key = keys[i]!;
    const slash = key.indexOf('/');
    if (slash <= 0) continue;
    const h = key.slice(0, slash);
    const s = key.slice(slash + 1);
    if (!s || !matchesHost(h)) continue;
    return {
      kind: 'session',
      hostId: h,
      sessionId: s,
      mentionKey: `${h}:${s}`,
      route: openChatHref(h, s),
    };
  }
  return undefined;
}

// Track route transitions (`hashchange`, `popstate`, and `@solidjs/router`'s
// `history.pushState` / `replaceState`) so jumping from a chat to `/board` or
// `/files` via `<A>` / `useNavigate()` arms a return-route automatically.
if (typeof window !== 'undefined') {
  let prevHash = typeof location !== 'undefined' ? location.hash : '';
  const onLocationChange = () => {
    const nextHash = typeof location !== 'undefined' ? location.hash : '';
    if (nextHash === prevHash) return;
    const prevChat = parseChatRoute(prevHash);
    const nextChat = parseChatRoute(nextHash);
    prevHash = nextHash;
    setRouteTick((n) => n + 1);
    if (nextChat) {
      setReturnRoute(null);
      return;
    }
    const nextPath = nextHash.startsWith('#') ? nextHash.slice(1) : nextHash;
    if (prevChat && returnRoute() === null && /^\/(board|files)(\/|$|\?)/.test(nextPath)) {
      setReturnRoute(prevChat.route);
    }
  };
  window.addEventListener('hashchange', onLocationChange);
  window.addEventListener('popstate', onLocationChange);
  if (typeof history !== 'undefined') {
    const origPush = history.pushState.bind(history);
    const origReplace = history.replaceState.bind(history);
    history.pushState = (...args) => {
      origPush(...args);
      onLocationChange();
    };
    history.replaceState = (...args) => {
      origReplace(...args);
      onLocationChange();
    };
  }
}

/** Navigates to `#/s/<host>/<session>`. */
export function openChat(target: Pick<ChatTarget, 'hostId' | 'session'>, _pcName = ''): void {
  setReturnRoute(null);
  appNavigate(openChatHref(target.hostId, target.session));
  setRouteTick((n) => n + 1);
}

/** Navigates to `#/new/<host>?cwd=`. */
export function openNewChat(hostId: string, cwd?: string): void {
  setReturnRoute(null);
  appNavigate(newChatHref(hostId, cwd));
  setRouteTick((n) => n + 1);
}

/** Compatibility accessor returning the active v2 session target (or `null` when none is open). */
export const openChatState = (): OpenChat | null => {
  const active = activeChatTargetFor();
  if (!active || active.kind !== 'session') return null;
  return { target: bareTarget(active.hostId, active.sessionId, ''), pcName: '' };
};

/** Whether a chat or draft route is currently shown. */
export const chatVisible = (): boolean => currentChatRoute() !== undefined;

export function updateOpenTarget(_target: ChatTarget): void {
  // v2 hub cards own session labels reactively.
}

/** Leaves an open chat route back to the list/home. */
export function hideChat(): void {
  if (!currentChatRoute()) return;
  appNavigate(desktopShell() ? '/' : '/chat');
  setRouteTick((n) => n + 1);
}

/** Navigates back to the most recent open chat, if any. */
export function showChat(): boolean {
  const active = activeChatTargetFor();
  if (!active) return false;
  appNavigate(active.route);
  setRouteTick((n) => n + 1);
  return true;
}

export function closeChat(): void {
  setReturnRoute(null);
}

/** Records the chat route a screen (Board, Files, Editor) left so closing it returns there. */
export function markReturnToChat(route?: string): void {
  const resolved = route ?? currentChatRoute() ?? activeChatTargetFor()?.route ?? '/';
  setReturnRoute(resolved);
}

/** Whether a screen left a chat with the return-route still armed. */
export function hasMarkedReturnToChat(): boolean {
  return returnRoute() !== null;
}

/** The armed return-route, if any. */
export function returnChatRoute(): string | null {
  return returnRoute();
}

/** Navigates back to the chat that opened Board/Files/Editor (once). */
export function resumeChatAfterTerminal(): boolean {
  const route = returnRoute();
  if (!route) return false;
  setReturnRoute(null);
  appNavigate(route);
  setRouteTick((n) => n + 1);
  return true;
}

export function clearReturnToChat(): void {
  setReturnRoute(null);
}

/**
 * Pure transition helper (UNI-312, unit-tested in `open.test.ts`).
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

export function syncChatVisibleOnScreenChange(_prevScreen: string | undefined, _nextScreen: string): void {
  // Route-driven in v2; kept for callers/tests.
}

// ── Chat → board portal (UNI-181) ────────────────────────────────────────

function goBoard(hostId?: string, cwd?: string): void {
  const back = currentChatRoute() ?? activeChatTargetFor(hostId)?.route ?? (hostId ? newChatHref(hostId, cwd) : '/');
  markReturnToChat(back);
  appNavigate('/board');
  setRouteTick((n) => n + 1);
}

/** Opens a task's detail pane on the board and returns to the chat when closed. */
export function openTaskFromChat(hostId: string, taskId: string, cwd?: string): void {
  requestBoardJump({ hostId, taskId, cwd, fromChat: true });
  goBoard(hostId, cwd);
}

/** Opens the board's New task sheet (project picked from the chat's cwd) and returns when closed. */
export function addTaskFromChat(hostId: string, cwd?: string): void {
  requestBoardJump({ hostId, newTask: true, cwd, fromChat: true });
  goBoard(hostId, cwd);
}

/** Opens the board with the current chat armed as the return route. */
export function openBoardFromChat(hostId?: string, cwd?: string): void {
  goBoard(hostId, cwd);
}
