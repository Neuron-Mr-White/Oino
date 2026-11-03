/**
 * The timeline's render context (WP-A6): one per conversation, created by
 * `Conversation` and threaded through `Timeline` → `TurnRow` → work items,
 * so rows never reach into globals and props stay stable for Solid memo
 * boundaries (§3.6 — no per-row closures in `<For>`).
 */
import type { RefChat } from '../../lib/chat/pathStat';
import type { CommandInfo } from '../../lib/session/types';
import type { SessionHandle } from '../../lib/session/store';
import type { HealCause } from '../../lib/timeline/chatHealth';

export type TimelineCtx = {
  /** The open session (cmds, outbox actions, lazy `get_entries`). */
  session: SessionHandle;
  hostId: string;
  /** Reactive: the host can swap the session file under an open stream. */
  sessionId: () => string;
  /** The session's project folder (tool subjects, path resolution). */
  cwd: () => string | undefined;
  /** The `RefChat` adapter for this session (stable identity — see `sessionRefChat`). */
  refs: RefChat;
  /** `/`-command names for bubble chips (`/model`, skills). */
  commands: () => ReadonlySet<string>;
  /** Column width for the markdown cache's width class. */
  columnWidth: () => number;
  /** Route to a session of this host (A7 Fork: the host streams the new id). */
  openSession?: (sessionId: string) => void;
  /**
   * Z-28's "Also undo file changes": supplied by WP-A27 once the
   * `unipi.file-summary` extension is enabled (D-1). Absent ⇒ hidden.
   */
  undoFiles?: UndoFiles;
  /** The cron scheduler started this chat (card `source: "cron"`): its first user
   * message is the job's prompt, marked "Scheduled run". Absent ⇒ a user chat. */
  cronRun?: () => boolean;
  /** chat-vanish: the host answers (the self-heal only acts then). Absent ⇒ assumed online. */
  hostOnline?: () => boolean;
  /** chat-vanish: re-open the stream / re-mount the list for `cause`, and log why. */
  heal?: (cause: HealCause) => void;
};

/** The file-summary capability the edit menu needs (WP-A27 fills it). */
export type UndoFiles = {
  /** This turn changed files the extension can restore. */
  canUndo(turnId: string): boolean;
  /** Restore this turn's files and every later turn's (newest first). */
  undoFrom(turnId: string): Promise<void>;
};

/** Non-builtin `/` command names for the bubble tokenizer. */
export function commandNames(commands: readonly CommandInfo[]): ReadonlySet<string> {
  return new Set(commands.filter((c) => c.source !== 'builtin').map((c) => c.name));
}
