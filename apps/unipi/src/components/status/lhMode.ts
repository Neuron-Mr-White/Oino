/**
 * UNI-269: the session's long-horizon working mode (Goal / Ralph / Swarm /
 * Graph, or plain) as the bridge sends it (`hello.lhMode`, `lh_mode` pushes,
 * docs/m5/PROTOCOL.md §7.1), plus the pure pieces of the chat's mode chip,
 * its popover and the chat list's mode badge.
 */
import { MODE_LABEL, type LhMode, type LhProgress, type LhRun } from './lhProgress';

/** `@pi-unipi/core` LhStateEvent, relayed verbatim. `mode: "none"` = plain. */
export type LhModeState = {
  mode: string;
  paused?: string;
  owner?: { kind: string; status: 'active' | 'parked' };
  lastStop?: { kind: string; at: number };
};

/** What `set_lh_mode` accepts; `none` = plain (/unipi:regular). */
export type LhModeChoice = LhMode | 'none';

export const LH_MODES: readonly LhMode[] = ['goal', 'ralph', 'swarm', 'graph'];

/** Short descriptions (long-horizon's own rubric, packages/long-horizon/src/modes.ts). */
export const MODE_DESCRIPTION: Record<LhModeChoice, string> = {
  goal: 'One objective pursued across many turns until it is verifiably true.',
  ralph: 'Works through a task file / checklist over many iterations.',
  swarm: 'Independent items run by parallel workers, then one synthesis.',
  graph: 'Multi-step work where later steps depend on earlier results.',
  none: 'Plain chat — no long-horizon machinery.',
};

export function isLhMode(value: unknown): value is LhMode {
  return typeof value === 'string' && (LH_MODES as readonly string[]).includes(value);
}

export function isLhModeState(value: unknown): value is LhModeState {
  return !!value && typeof value === 'object' && typeof (value as { mode?: unknown }).mode === 'string';
}

/** A mode id's label; unknown ids (a newer unipi) pass through, capitalised. */
export function modeLabel(mode: string): string {
  if (isLhMode(mode)) return MODE_LABEL[mode];
  if (mode === 'none') return 'Plain';
  return mode ? mode[0]!.toUpperCase() + mode.slice(1) : '';
}

/** What the chip shows: the working mode (or a parked one), or nothing in plain mode. */
export type ModeView = {
  /** Mode id (`goal`…; may be an unknown future id). */
  mode: string;
  label: string;
  /** `active` = its owner runs the session · `selected` = sticky, no work started yet · `paused` = parked owner while plain. */
  phase: 'active' | 'selected' | 'paused';
  /** The live run of this mode, when the progress snapshot has one. */
  run?: LhRun;
  /** The run is going right now (pulse). */
  live: boolean;
};

/**
 * The chip's view. `lhMode` wins (newer bridges); without it (older
 * bridges) the progress snapshot's current run stands in, so the chip
 * still shows a running Graph etc.
 */
export function modeView(lhMode: LhModeState | undefined, progress: LhProgress | undefined): ModeView | undefined {
  const current = progress?.current;
  const runOf = (mode: string) => (current && current.mode === mode ? current : undefined);
  if (lhMode) {
    if (lhMode.mode && lhMode.mode !== 'none') {
      const run = runOf(lhMode.mode);
      const active = lhMode.owner?.status === 'active' || run?.status === 'running';
      return { mode: lhMode.mode, label: modeLabel(lhMode.mode), phase: active ? 'active' : 'selected', run, live: run?.status === 'running' };
    }
    if (lhMode.paused && lhMode.paused !== 'none') {
      return { mode: lhMode.paused, label: modeLabel(lhMode.paused), phase: 'paused', run: runOf(lhMode.paused), live: false };
    }
    return undefined;
  }
  if (current && current.status === 'running') return { mode: current.mode, label: MODE_LABEL[current.mode], phase: 'active', run: current, live: true };
  if (current && current.status === 'paused') return { mode: current.mode, label: MODE_LABEL[current.mode], phase: 'paused', run: current, live: false };
  return undefined;
}

/** The chip's compact count (`3/7`, `#4`, `45%`), or undefined. */
export function modeChipCount(view: ModeView | undefined): string | undefined {
  const run = view?.run;
  if (!run) return undefined;
  if (run.mode === 'ralph' && run.ralph) return run.ralph.total > 0 ? `${run.ralph.checked}/${run.ralph.total}` : `#${run.ralph.iteration}`;
  if (run.mode === 'goal') return run.goal?.percent !== undefined ? `${Math.round(run.goal.percent)}%` : undefined;
  return run.counts.total > 0 ? `${run.counts.done + run.counts.failed}/${run.counts.total}` : undefined;
}

/** One plain line of live state for the popover. */
export function modeLiveText(run: LhRun): string {
  if (run.mode === 'ralph' && run.ralph) {
    const max = run.ralph.maxIterations > 0 ? `/${run.ralph.maxIterations}` : '';
    const items = run.ralph.total > 0 ? ` · ${run.ralph.checked}/${run.ralph.total} checked` : '';
    return `Iteration ${run.ralph.iteration}${max}${items}`;
  }
  if (run.mode === 'goal' && run.goal) {
    const max = run.goal.maxTurns > 0 ? `/${run.goal.maxTurns}` : '';
    return `Turn ${run.goal.turn}${max}${run.goal.percent !== undefined ? ` · ${Math.round(run.goal.percent)}% done` : ''}`;
  }
  const c = run.counts;
  if (c.total === 0) return 'No items yet';
  const parts = [`${c.done + c.failed}/${c.total} settled`];
  if (c.failed) parts.push(`${c.failed} failed`);
  if (c.running) parts.push(`${c.running} running`);
  return parts.join(' · ');
}

/** 0..100 for the popover's bar, or undefined when there's nothing to measure. */
export function modePercent(run: LhRun): number | undefined {
  if (run.mode === 'goal') return run.goal?.percent;
  if (run.mode === 'ralph') return run.ralph && run.ralph.total > 0 ? Math.round((run.ralph.checked / run.ralph.total) * 100) : undefined;
  return run.counts.total > 0 ? Math.round(((run.counts.done + run.counts.failed) / run.counts.total) * 100) : undefined;
}

/** The phase word the popover shows next to the mode. */
export const PHASE_WORD: Record<ModeView['phase'], string> = { active: 'Running', selected: 'On — next message runs in it', paused: 'Paused' };

/**
 * What picking `to` in the popover does, as a short hint — so switching
 * away from running work is never a surprise. `undefined` = no extra note.
 */
export function switchHint(view: ModeView | undefined, to: LhModeChoice): string | undefined {
  if (!view || view.phase !== 'active') return undefined;
  if (to === view.mode) return undefined;
  const what = view.mode === 'ralph' ? 'loop' : view.label.toLowerCase();
  return to === 'none' ? `Stops the ${what}` : `Pauses the ${what}`;
}

/** The app's error text for a failed `set_lh_mode` (an older bridge answers "unknown type"). */
export function modeErrorText(message: string): string {
  return /unknown type/i.test(message) ? 'Switching modes from the app needs a newer unipi on this PC.' : message;
}

/** The list's mode badge for a row (session card lh_mode / lh_paused). */
export function listModeBadge(lhMode: string | undefined, lhPaused: string | undefined): { mode: string; label: string; paused: boolean } | undefined {
  if (lhMode && lhMode !== 'none') return { mode: lhMode, label: modeLabel(lhMode), paused: false };
  if (lhPaused && lhPaused !== 'none') return { mode: lhPaused, label: `${modeLabel(lhPaused)} · paused`, paused: true };
  return undefined;
}
