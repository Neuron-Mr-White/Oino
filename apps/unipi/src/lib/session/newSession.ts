/**
 * Start and close pi chats from the app (UNI-260; docs/m5/PROTOCOL.md §10).
 *
 * - `newChatSession` → Tauri `chat_new_session` → host `chat_new`: a herdr
 *   tab (existing / new workspace) or a tmux window/session in a folder,
 *   running the PC's launcher; answers once the pi's bridge is up.
 * - `stopChat` → `chat_stop` → host `chat_close`: graceful stop (+ close the
 *   tab/window).
 * - `chatConfig` → the PC's launcher (Settings → PCs) + which muxes it has.
 * - Hidden chats ("Just hide from list"), per PC, in localStorage.
 * - Pure helpers for the New chat sheet (where options, folder chips).
 */
import { createSignal } from 'solid-js';
import {
  ChatCloseDataSchema,
  ChatConfigDataSchema,
  ChatNewDataSchema,
  type AppError,
  type ChatNewData,
  type ChatNewTarget,
  type TreeData,
} from '@unipi/protocol';
import { call, type Result } from '../ipc';
import type { ChatTarget } from './list';

export type { ChatNewData, ChatNewTarget } from '@unipi/protocol';

export function newChatSession(hostId: string, target: ChatNewTarget, cwd?: string): Promise<Result<ChatNewData>> {
  return call('chat_new_session', { hostId, target, cwd }, ChatNewDataSchema);
}

/** Where "new chat beside this one" starts: a new tab in the chat's own herdr
 * workspace (a new tmux window for tmux chats) — not a new workspace. */
export function hereTarget(t: Pick<ChatTarget, 'mux' | 'session' | 'workspaceId'>): ChatNewTarget {
  return t.mux === 'tmux' ? { kind: 'tmux_window', session: t.session } : { kind: 'herdr_workspace', session: t.session, workspace_id: t.workspaceId };
}

/** A real host path to start in: the list shortens `/home/<u>` to `~`, which
 * the host can't resolve, so only absolute paths (Unix or `C:\`) are passed;
 * otherwise the host picks its default folder. */
export function hereCwd(cwd: string | undefined): string | undefined {
  return cwd && (cwd.startsWith('/') || /^[A-Za-z]:[\\/]/.test(cwd)) ? cwd : undefined;
}

/** One tap "New chat in this workspace": a separate pi beside `t`, in the same folder. */
export function newChatHere(t: Pick<ChatTarget, 'hostId' | 'mux' | 'session' | 'workspaceId'>, cwd: string | undefined): Promise<Result<ChatNewData>> {
  return newChatSession(t.hostId, hereTarget(t), hereCwd(cwd));
}

export function stopChat(hostId: string, pid: number, closeTab: boolean) {
  return call('chat_stop', { hostId, pid, closeTab }, ChatCloseDataSchema);
}

export function chatConfig(hostId: string, launcher?: string) {
  return call('chat_config', launcher === undefined ? { hostId } : { hostId, launcher }, ChatConfigDataSchema);
}

/** An older host (no chat_new / chat_close / chat_config) answers E_SCHEMA "unknown variant". */
export function isOlderHost(error: AppError | undefined): boolean {
  return !!error && error.code === 'E_SCHEMA' && /unknown variant|chat_(new|close|config)/i.test(String(error.message));
}

// ── "New chat" sheet open state (Chat list +, desktop palette) ───────────

const [newOpen, setNewOpen] = createSignal(false);
export const newChatOpen = newOpen;
export const setNewChatOpen = (open: boolean) => setNewOpen(open);

// ── Hidden chats, per PC ─────────────────────────────────────────────────

const HIDDEN = (hostId: string) => `unipi.chat.hidden.${hostId}`;
const [hiddenRev, setHiddenRev] = createSignal(0);

/** A chat's hide key: its pane + pid (a new pi in that pane shows again). */
export function hideKey(t: Pick<ChatTarget, 'paneId' | 'pid'>): string {
  return `${t.paneId}#${t.pid}`;
}

export function readHidden(hostId: string): string[] {
  hiddenRev();
  try {
    const raw = JSON.parse(localStorage.getItem(HIDDEN(hostId)) ?? '[]');
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function writeHidden(hostId: string, keys: string[]): void {
  try {
    localStorage.setItem(HIDDEN(hostId), JSON.stringify(keys.slice(-200)));
  } catch {
    // ignore
  }
  setHiddenRev((n) => n + 1);
}

export function hideChatFromList(hostId: string, t: Pick<ChatTarget, 'paneId' | 'pid'>): void {
  const key = hideKey(t);
  const keys = readHidden(hostId);
  if (!keys.includes(key)) writeHidden(hostId, [...keys, key]);
}

export function unhideChat(hostId: string, key: string): void {
  writeHidden(hostId, readHidden(hostId).filter((k) => k !== key));
}

/** Drops hide keys whose pane/pid is gone (keeps the list small). */
export function pruneHidden(hostId: string, live: Array<Pick<ChatTarget, 'paneId' | 'pid'>>): void {
  const alive = new Set(live.map(hideKey));
  const keys = readHidden(hostId);
  const kept = keys.filter((k) => alive.has(k));
  if (kept.length !== keys.length) writeHidden(hostId, kept);
}

// ── Where a new chat can go ──────────────────────────────────────────────

export type WhereOption = {
  /** Stable id for the radio list. */
  id: string;
  target: ChatNewTarget;
  title: string;
  sub: string;
  /** Wants a name (new workspace / tmux session). */
  needsName: boolean;
  /** The folder this place already uses (a workspace's focused pane). */
  cwd?: string;
};

/** Every place a chat can start on this PC, best first: existing herdr
 *  workspaces (non-protected running sessions), a new herdr workspace per
 *  session, then tmux sessions + a new tmux session when tmux is there. */
export function whereOptions(tree: TreeData | undefined, opts: { herdr: boolean; tmux: boolean }): WhereOption[] {
  const out: WhereOption[] = [];
  const protectedNames = tree?.protected ?? [];
  const herdrSessions = tree?.herdr.available ? (tree.herdr.sessions ?? []).filter((s) => s.running && !protectedNames.includes(s.name)) : [];
  if (opts.herdr) {
    for (const s of herdrSessions) {
      for (const ws of s.workspaces) {
        const pane = ws.tabs.flatMap((t) => t.panes).find((p) => p.focused) ?? ws.tabs[0]?.panes[0];
        out.push({
          id: `hw:${s.name}:${ws.id}`,
          target: { kind: 'herdr_workspace', session: s.name, workspace_id: ws.id },
          title: ws.label || `Workspace ${ws.number}`,
          sub: `New tab · herdr${herdrSessions.length > 1 ? ` ${s.name}` : ''}`,
          needsName: false,
          cwd: pane?.cwd,
        });
      }
    }
    for (const s of herdrSessions) {
      out.push({
        id: `hn:${s.name}`,
        target: { kind: 'herdr_new_workspace', session: s.name },
        title: 'New workspace',
        sub: `herdr${herdrSessions.length > 1 ? ` ${s.name}` : ''}`,
        needsName: true,
      });
    }
  }
  if (opts.tmux) {
    const tmuxSessions = tree?.tmux.available ? (tree.tmux.sessions ?? []).filter((s) => !protectedNames.includes(s.name)) : [];
    for (const s of tmuxSessions) {
      const pane = s.windows.find((w) => w.active)?.panes.find((p) => p.active);
      out.push({
        id: `tw:${s.name}`,
        target: { kind: 'tmux_window', session: s.name },
        title: s.name,
        sub: 'New window · tmux',
        needsName: false,
        cwd: pane?.cwd,
      });
    }
    out.push({ id: 'tn', target: { kind: 'tmux_new_session', name: 'pi' }, title: 'New tmux session', sub: 'tmux', needsName: true });
  }
  return out;
}

/** Recent folders for the folder chips: pane cwds of running pis first, then
 *  other pane cwds, then the Files roots — distinct, home shortened, ≤ 8. */
export function folderChoices(tree: TreeData | undefined, roots: readonly string[] = [], home?: string): string[] {
  const pi: string[] = [];
  const other: string[] = [];
  const add = (list: string[], cwd?: string) => {
    if (cwd && !pi.includes(cwd) && !other.includes(cwd)) list.push(cwd);
  };
  for (const s of tree?.herdr.available ? tree.herdr.sessions ?? [] : []) {
    for (const ws of s.workspaces) for (const t of ws.tabs) for (const p of t.panes) add(p.agent?.name === 'pi' ? pi : other, p.cwd);
  }
  for (const s of tree?.tmux.available ? tree.tmux.sessions ?? [] : []) {
    for (const w of s.windows) for (const p of w.panes) add(p.agent ? pi : other, p.cwd);
  }
  const all = [...pi, ...other];
  for (const r of roots) if (!all.includes(r)) all.push(r);
  return all.filter((d) => d !== home || all.length === 1).slice(0, 8);
}

/** `/home/u/proj` → `~/proj` (display only). */
export function shortPath(path: string, home?: string): string {
  if (home && (path === home || path.startsWith(`${home}/`))) return `~${path.slice(home.length)}`;
  return path.replace(/^\/home\/[^/]+/, '~');
}

/** Last path segment (the new workspace's default name). */
export function folderName(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/');
  const last = parts[parts.length - 1];
  return last && last !== '~' ? last : 'pi';
}

/** The ChatTarget the chat layer opens for a just-started pi. */
export function targetFromNew(hostId: string, data: ChatNewData, title: string, label: string): ChatTarget {
  return {
    hostId,
    session: data.session,
    workspaceId: data.workspace_id ?? '',
    workspaceLabel: label,
    tabId: data.tab_id ?? '',
    paneId: data.pane_id,
    pid: data.pid,
    title,
    cwd: data.cwd ? shortPath(data.cwd) : undefined,
    status: 'idle',
    focused: false,
    ...(data.mux === 'tmux' ? { mux: 'tmux' as const } : {}),
  };
}

/** Plain words for a failed start/stop. */
export function errorWords(error: AppError): { text: string; hint?: string } {
  if (isOlderHost(error)) return { text: 'This PC runs an older UniPi.', hint: 'Update UniPi on the PC (re-run the installer), then try again.' };
  return { text: String(error.message), hint: error.hint };
}
