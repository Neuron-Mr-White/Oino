/**
 * Ctrl+B / ⌘B (P0-10): the shell's sidebar chord also does the old harness's
 * subagent backgrounding. When the focused session is waiting on a
 * **foreground** subagent — a live `run_subagent` / `read_subagent` call
 * (a background run returns at once, so a call still in flight is a
 * foreground wait) — the chord invokes the subagents extension's
 * `unipi.subagents.foreground` shortcut (every foreground run → background);
 * otherwise it toggles the sidebar. The sticky `unipi:subagents:list` topic
 * is not consulted: it is not per session, and a stale record must never
 * eat the sidebar chord.
 */
import type { SessionState } from '../session/types';

export const SUBAGENTS_SHORTCUT = 'unipi.subagents.foreground';
const WAIT_TOOLS = new Set(['run_subagent', 'read_subagent']);

type Pick_ = Pick<SessionState, 'liveTools'>;

/** Is a foreground subagent holding the session? (synchronous: no round trip on the key press) */
export function hasForegroundSubagent(state: Pick_ | undefined): boolean {
  if (!state) return false;
  return Object.values(state.liveTools ?? {}).some((tool) => WAIT_TOOLS.has(tool.name) && !tool.done);
}
