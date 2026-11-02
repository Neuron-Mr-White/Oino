/**
 * Settings actions stay in Settings (owner5 #5/#12): a Manage row or a form
 * `action` field runs its extension command without leaving Settings.
 *
 * - `background` — commands that just do something (sync, reload, a status
 *   report): run quietly, the reply shows inline on the row.
 * - `sheet` — commands that need UI (a picker, a wizard): a Settings sheet
 *   renders the command's dialog steps (`ask_user` / select / input…) and
 *   its replies.
 * - `chat` — commands about one chat (consolidate this session, rename it):
 *   the row says "Opens in chat ↗" and opens the chat explicitly.
 *
 * An extension command needs a session to run in (there is no host op that
 * runs one headless): the shown chat, else the most recent open one, else
 * this PC's latest chat — opened in the background, never navigated to — or
 * a new empty one. Its replies are claimed (`claimReplies`) so they land on
 * the row or sheet, not as a toast or the floating report panel.
 */
import { createRoot, createSignal } from 'solid-js';
import { createSession, sessions, type SessionHandle } from '../session/store';
import { hostClient } from '../host/factory';
import type { HostClient } from '../host/client';
import { markInternalSession } from '../session/internal';
import { listExtensions } from '../../settings/hostCmd';
import { newSession } from '../session/newSession';
import { claimReplies, noteCommandRun } from '../reports';
import type { ToastKind } from '../toasts';
import { installUiSinks } from '../../ui/registries';
import { installUiEffects } from '../../ui/slots/effects';
import { chatHasCommand } from './manage';

export type CommandMode = 'background' | 'sheet' | 'chat';

/** A command Settings answers natively (owner5 #17: `/agents` → the Agent profiles page). */
const overrides = new Map<string, () => void>();
const [overrideRev, setOverrideRev] = createSignal(0);

export function registerCommandOverride(command: string, open: () => void): () => void {
  overrides.set(command, open);
  setOverrideRev((n) => n + 1);
  return () => {
    if (overrides.get(command) === open) overrides.delete(command);
    setOverrideRev((n) => n + 1);
  };
}

export function commandOverride(command: string): (() => void) | undefined {
  overrideRev();
  return overrides.get(command.trim().replace(/^\//, ''));
}

/** Commands that just do something and answer with one notice or report. */
const BACKGROUND = new Set([
  'mcp-sync',
  'mcp-reload',
  'mcp-status',
  'global-memory-list',
  'dream status',
  'dream report',
  'dream run',
  'web-cache-clear',
  'notify-test',
]);
/** Commands about one chat: they run in it, visibly. */
const CHAT = new Set(['memory-consolidate', 'rename-now']);

/** How Settings runs `command` (anything unknown gets the sheet: it can show any step). */
export function commandMode(command: string): CommandMode {
  const cmd = command.trim().replace(/^\//, '');
  if (BACKGROUND.has(cmd)) return 'background';
  if (CHAT.has(cmd)) return 'chat';
  return 'sheet';
}

/** Owner of a command (the extension id), for the "extension is off" state. */
const COMMAND_OWNER: Array<[RegExp, string]> = [
  [/^mcp(-|$)/, 'unipi.mcp'],
  [/^(global-)?memory(-|$)/, 'unipi.memory'],
  [/^dream(\s|$)/, 'unipi.dream'],
  [/^skills(-|$)/, 'unipi.skill-registry'],
  [/^agents$/, 'unipi.subagents'],
  [/^web-cache/, 'unipi.web-api'],
  [/^notify-/, 'unipi.notify'],
  [/^rename-now$/, 'unipi.utility'],
];

export function commandOwner(command: string): string | undefined {
  const cmd = command.trim().replace(/^\//, '');
  return COMMAND_OWNER.find(([re]) => re.test(cmd))?.[1];
}

/** Resolves once the chat lists its commands, or soon after it is live without any, or after `ms`. */
export function commandsReady(handle: { state: { commands: ReadonlyArray<unknown>; conn?: string } }, ms = 6000): Promise<void> {
  return new Promise((resolve) => {
    const start = Date.now();
    let liveAt: number | undefined;
    const tick = () => {
      const now = Date.now();
      if (handle.state.conn === 'live') liveAt ??= now;
      if (handle.state.commands.length > 0 || now - start > ms || (liveAt !== undefined && now - liveAt > 1200)) resolve();
      else setTimeout(tick, 80);
    };
    tick();
  });
}

/**
 * Settings' own command session per host: a fresh `session_new` runtime that
 * is never shown and never touches the user's chats (a command run there
 * adds no rows to any conversation). The stream opens as `new` every time
 * (a session with no message has no file to reopen by id).
 */
const scratch = new Map<string, SessionHandle>();
let scratchSeq = 0;

function scratchSession(hostId: string): SessionHandle {
  const hit = scratch.get(hostId);
  if (hit && hit.state.conn !== 'closed') return hit;
  hit?.close();
  const real = hostClient(hostId);
  const client: HostClient = Object.create(real, {
    openSession: {
      value: (_req: unknown, sink: Parameters<HostClient['openSession']>[1]) =>
        real.openSession({ kind: 'new' }, {
          ...sink,
          snapshot: (snap) => {
            // Never listed with the user's chats.
            const sid = (snap as { session_id?: unknown }).session_id;
            if (typeof sid === 'string' && sid) markInternalSession(sid);
            sink.snapshot(snap);
          },
          events: (items) => sink.events(items),
          closed: (error) => sink.closed(error),
        }),
    },
  });
  const handle = createRoot(() => createSession(hostId, `settings-${Date.now().toString(36)}-${++scratchSeq}`, { client }));
  scratch.set(hostId, handle);
  return handle;
}

/** Tests: drop the command sessions. */
export function resetSettingsCommandSessions(): void {
  for (const h of scratch.values()) h.close();
  scratch.clear();
}

/** The session a Settings command runs in — Settings' own, never a chat of the user's. */
export async function settingsCommandChat(hostId: string, _start: typeof newSession = newSession): Promise<SessionHandle | undefined> {
  installUiSinks();
  installUiEffects();
  const handle = scratchSession(hostId);
  const deadline = Date.now() + 20_000;
  while (handle.state.conn !== 'live' && Date.now() < deadline) {
    if (handle.state.conn === 'closed') return undefined;
    await new Promise((r) => setTimeout(r, 80));
  }
  return handle.state.conn === 'live' ? handle : undefined;
}

export type Reply = { text: string; kind: ToastKind };

export type StartedCommand =
  | { ok: true; handle: SessionHandle; release: () => void }
  | { ok: false; reason: 'no-chat' | 'missing' };

/**
 * Sends `/<command>` in the Settings command chat; every reply of that chat
 * goes to `onReply` until `release()` (a Settings row or sheet owns them).
 */
export async function startSettingsCommand(
  command: string,
  opts: { hostId: string; hidden?: boolean; onReply: (reply: Reply) => void; start?: typeof newSession },
): Promise<StartedCommand> {
  const handle = await settingsCommandChat(opts.hostId, opts.start);
  if (!handle) return { ok: false, reason: 'no-chat' };
  await commandsReady(handle);
  if (handle.state.commands.length > 0 && !chatHasCommand(handle, command, opts.hidden)) {
    // Runner commands are often hidden (not listed): only an extension that is off is "missing".
    const owner = commandOwner(command);
    const list = owner ? await listExtensions(opts.hostId).catch(() => undefined) : undefined;
    const ext = list?.ok ? list.value.find((e) => e.id === owner) : undefined;
    if (!owner || !ext || !ext.enabled) return { ok: false, reason: 'missing' };
  }
  const release = claimReplies(handle.key, (text, kind) => opts.onReply({ text, kind }));
  noteCommandRun(handle.key, command);
  handle.send(`/${command}`, { mode: 'prompt' });
  return { ok: true, handle, release };
}

export type BackgroundOutcome =
  | { kind: 'done'; replies: Reply[] }
  | { kind: 'quiet' }
  | { kind: 'no-chat' }
  | { kind: 'missing' };

/**
 * Runs a background command: resolves with its first reply (plus any that
 * follow within `settleMs`), or `quiet` when none came within `waitMs`.
 */
export async function runInBackground(
  command: string,
  opts: { hostId: string; hidden?: boolean; waitMs?: number; settleMs?: number; start?: typeof newSession; onProgress?: (replies: Reply[]) => void },
): Promise<BackgroundOutcome> {
  const replies: Reply[] = [];
  let wake: (() => void) | undefined;
  const started = await startSettingsCommand(command, {
    ...opts,
    onReply: (reply) => {
      replies.push(reply);
      opts.onProgress?.([...replies]);
      wake?.();
    },
  });
  if (!started.ok) return { kind: started.reason };
  try {
    await new Promise<void>((resolve) => {
      const total = setTimeout(resolve, opts.waitMs ?? 90_000);
      let quiet: ReturnType<typeof setTimeout> | undefined;
      const done = () => {
        clearTimeout(total);
        resolve();
      };
      // Late lines of the same answer ("syncing…" then "synced 12 servers") still
      // count: wait until the command has been quiet for a while.
      wake = () => {
        clearTimeout(quiet);
        quiet = setTimeout(done, opts.settleMs ?? 8000);
      };
      if (replies.length) wake();
    });
  } finally {
    started.release();
  }
  return replies.length ? { kind: 'done', replies } : { kind: 'quiet' };
}
