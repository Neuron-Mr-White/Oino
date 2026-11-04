/**
 * The ONE working line above the composer (owner 2026-11-04: "at first glance it doesn't look
 * like the session is running"). Pure: the activity model (`lib/session/activity`) decides the
 * state, this picks the words.
 *
 * - `needs-you`: an `ask_user` question waits (the merged "Answer ↑" chip) — accent, still.
 * - `working`: the agent runs, or the idle session waits on work that will wake it — animated.
 * - `background`: quiet work without a wake (a background task) — still, dim.
 */
import { liveBusy } from '../session/activity';
import type { RunInfo } from '../session/types';

export type WorkingLineKind = 'needs-you' | 'working' | 'background';
export type WorkingLineModel = {
  kind: WorkingLineKind;
  /** The headline ("Working…", "Waiting on 1 subagent", the extension's working message). */
  text: string;
  /** Epoch ms the clock counts from (working only). */
  since?: number;
  /** Running work a click opens in the Running tab. */
  openRunning: boolean;
};

const list = (v: readonly string[] | undefined): string => (v ?? []).join(' · ');

/** The extension-set working message's words, key hints ("Ctrl+B to …") dropped. */
export function workingHead(msg: string | undefined): string | undefined {
  if (!msg) return undefined;
  const isHint = (p: string) => /\b(?:Ctrl|Cmd|Alt|Shift|Esc)\b(?:\+\S+)?/.test(p);
  const head = msg.split(' · ').map((p) => p.trim()).filter((p) => p && !isHint(p)).join(' · ');
  return head || undefined;
}

export function workingLineOf(input: {
  run: RunInfo;
  /** The `ask_user` question text when one waits in the chat ("N questions waiting" for more). */
  ask?: string;
  /** The Running tab's live summary ("Subagents 1 running"), if any. */
  tray?: string;
  /** `setWorkingMessage` from the host / extensions. */
  message?: string;
}): WorkingLineModel | undefined {
  const { run } = input;
  if (input.ask) return { kind: 'needs-you', text: input.ask, openRunning: false };
  if (liveBusy(run)) {
    const since = run.startedAt;
    if (run.phase === 'compacting') return { kind: 'working', text: 'Compacting…', since, openRunning: !!input.tray };
    if (run.phase === 'retrying') return { kind: 'working', text: 'Retrying…', since, openRunning: !!input.tray };
    if (!run.running) {
      const on = list(run.waitingOn);
      return { kind: 'working', text: on ? `Waiting on ${on}` : (workingHead(input.message) ?? 'Waiting…'), since, openRunning: true };
    }
    return { kind: 'working', text: workingHead(input.message) ?? 'Working…', since, openRunning: !!input.tray };
  }
  const bg = list(run.backgroundOn);
  if (bg || input.tray) return { kind: 'background', text: bg ? `In the background: ${bg}` : (input.tray as string), openRunning: true };
  return undefined;
}
