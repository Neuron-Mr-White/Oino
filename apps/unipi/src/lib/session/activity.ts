/**
 * Session activity states (owner 2026-11-04, [status]) — ONE definition every surface shares:
 * the sidebar row, the phone Chats list, the header mode chip, the tray / mascot counts.
 *
 * - `needs-you` (blocking, accent): a dialog waits on the user — ask_user, a permission
 *   approval, any other interruption. Wins over everything.
 * - `working` (spinner + "Working"): the agent streams or runs tools (compaction and retries
 *   included), OR it waits on background work that will wake it: a background task with
 *   wake/notify, background subagents, graph / swarm nodes (host `pending_work`, core
 *   `waiting`).
 * - `background` (quiet marker, not "Working"): background tasks run WITHOUT a wake (host
 *   `background_work`, core `backgroundOn`).
 * - `idle` otherwise.
 *
 * The host card reports working even between turns; a live stream adds dialogs and run details.
 * Pure — no store access here.
 */
import type { SessionCard } from '../host/client';
import type { SessionState } from './types';

export type Activity = 'needs-you' | 'working' | 'background' | 'idle';

/** The activity plus what it is about (the waiting / background reasons), for tooltips. */
export type ActivityFacts = { activity: Activity; detail?: string };

/** The fields of an open session the reconciliation reads. */
export type LiveSession = Pick<SessionState, 'conn' | 'run' | 'dialogs'>;

const text = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
const joined = (list: readonly string[] | undefined): string | undefined => (list?.length ? list.join(' · ') : undefined);

/** The activity a host card reports on its own. */
export function cardActivity(card: SessionCard): ActivityFacts {
  if (card.needs_you) return { activity: 'needs-you' };
  const pending = text(card.pending_work);
  if (card.run_state === 'working' || pending) return { activity: 'working', ...(pending ? { detail: pending } : {}) };
  const background = text(card.background_work);
  if (background) return { activity: 'background', detail: background };
  return { activity: 'idle' };
}

/** True while the open session's own run state says it works or waits on wake-capable work. */
export function liveBusy(run: SessionState['run']): boolean {
  return run.running || run.phase === 'running' || run.phase === 'compacting' || run.phase === 'retrying' || run.phase === 'waiting' || !!run.waitingOn?.length;
}

/**
 * Reconcile the host card with the open stream. Either can report working: an idle stream
 * may be between turns while the host waits on wake-capable work. Dialogs mean "needs you".
 */
export function liveActivity(card: SessionCard, open: LiveSession | undefined): ActivityFacts {
  if (card.needs_you || (open?.conn === 'live' && open.dialogs.length > 0)) return { activity: 'needs-you' };
  if (open?.conn !== 'live') return cardActivity(card);
  const run = open.run;
  if (liveBusy(run)) {
    const detail = run.running ? undefined : joined(run.waitingOn);
    return { activity: 'working', ...(detail ? { detail } : {}) };
  }
  // An idle stream can be between turns; the host knows about wake-capable work.
  const pending = text(card.pending_work);
  if (card.run_state === 'working' || pending) return { activity: 'working', ...(pending ? { detail: pending } : {}) };
  const background = joined(run.backgroundOn) ?? text(card.background_work);
  if (background) return { activity: 'background', detail: background };
  return { activity: 'idle' };
}

/** Counts for the tray and the mascot: sessions that need you / work (background is quiet). */
export function activityCounts(list: readonly ActivityFacts[]): { needs: number; working: number; background: number } {
  let needs = 0;
  let working = 0;
  let background = 0;
  for (const f of list) {
    if (f.activity === 'needs-you') needs += 1;
    else if (f.activity === 'working') working += 1;
    else if (f.activity === 'background') background += 1;
  }
  return { needs, working, background };
}

/** The one word each state reads (the background marker has no word on the row). */
export const ACTIVITY_WORD: Record<Activity, string> = {
  'needs-you': 'Needs you',
  working: 'Working',
  background: 'Background',
  idle: '',
};

/** The tooltip of a state: its word and what it is about. */
export function activityTitle(facts: ActivityFacts): string | undefined {
  if (facts.activity === 'idle') return undefined;
  if (facts.activity === 'background') return facts.detail ? `Running in the background: ${facts.detail}` : 'Running in the background';
  if (facts.activity === 'working' && facts.detail) return `Working — waiting on ${facts.detail}`;
  return ACTIVITY_WORD[facts.activity];
}
