/**
 * Goal and ralph views ([lh-ui]) in the same design system as the graph: header (name, state
 * chip), segmented bar, facts list, an error card with Retry, and the connected timeline of the
 * events this view observed. Data: the LH_PROGRESS topic (`summary.goal` / `summary.ralph`).
 */
import { createEffect, createMemo, createSignal, Show } from 'solid-js';
import { RotateCcw, Loader2 } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import type { LhSummary } from '../../lib/status/lh';
import { LH_EXTENSION_ID } from '../../lib/status/lh';
import { refusalText, emptyCounts, type LhTimelineEntry } from '../../lib/status/lhGraph';
import { loopSnap, observeLoop, type LoopSnap } from '../../lib/status/lhLoop';
import { serviceCall } from '../session/modalData';
import { LhBar, LhTimeline } from './LhParts';
import './lh.css';

const GOAL_TONE: Record<string, string> = { active: 'run', errored: 'error', met: 'ok', done: 'ok', completed: 'ok', failed: 'fail' };

export function LhLoop(props: { s: SessionHandle; summary: LhSummary; onToast?: (text: string) => void }) {
  const [log, setLog] = createSignal<LhTimelineEntry[]>([]);
  let prev: LoopSnap | undefined;
  const snap = createMemo(() => loopSnap(props.summary));
  createEffect(() => {
    const next = snap();
    setLog((l) => observeLoop(l, prev, next, Date.now()));
    prev = next;
  });
  const [busy, setBusy] = createSignal(false);
  const goal = () => props.summary.goal;
  const ralph = () => props.summary.ralph;
  const errored = () => goal()?.status === 'errored';
  const retry = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      const reply = await serviceCall<unknown>(props.s, 'long-horizon.graph', { op: 'retry', target: 'goal' }, LH_EXTENSION_ID);
      props.onToast?.(refusalText(reply) ?? 'Continuing');
    } catch {
      void props.s.cmd({ type: 'prompt', message: `/${props.summary.mode} continue` }).then(() => props.onToast?.('Continuing')).catch(() => props.onToast?.('Could not continue'));
    } finally {
      setBusy(false);
    }
  };
  // Goal bar: turns of max; ralph: checked of total.
  const counts = () => {
    const c = emptyCounts();
    const g = goal();
    const r = ralph();
    if (g && g.maxTurns > 0) {
      c.completed = Math.min(g.turn, g.maxTurns);
      c.queued = Math.max(0, g.maxTurns - g.turn);
      if (g.status === 'errored') c.errored = 1;
    } else if (r && r.total > 0) {
      c.completed = r.checked;
      c.queued = Math.max(0, r.total - r.checked);
    }
    return c;
  };
  const timeline = () => [...log()].reverse();
  const state = () => (goal() ? goal()!.status : 'active');
  return (
    <div class="lh-view" data-testid={goal() ? 'lh-goal-view' : 'lh-ralph-view'}>
      <header class="lh-head">
        <div class="lh-head-top">
          <span class="lh-head-title" title={goal()?.objective ?? ralph()?.name}>{goal()?.objective ?? `Loop “${ralph()?.name}”`}</span>
          <span class="lh-chip lh-outcome" data-tone={GOAL_TONE[state()] ?? 'off'} data-testid="lh-loop-state">{errored() ? 'Error' : state()}</span>
        </div>
        <div class="lh-head-count">
          <Show when={goal()}>{(g) => <><strong>Turn {g().turn}</strong>{g().maxTurns > 0 ? ` of ${g().maxTurns}` : ''}</>}</Show>
          <Show when={ralph()}>{(r) => <><strong>Iteration {r().iteration}</strong><span class="lh-count" data-status="completed">{r().checked}/{r().total} checked</span></>}</Show>
        </div>
        <Show when={counts().completed + counts().queued > 0}>
          <LhBar counts={counts()} label={goal() ? 'Goal turns' : 'Checklist'} />
        </Show>
      </header>
      <Show when={props.summary.todosLine}>{(t) => <p class="lh-detail-line" data-testid="lh-loop-todos">{t()}</p>}</Show>
      <Show when={errored()}>
        <div class="lh-errcard" data-testid="lh-loop-error">
          <p class="lh-row-error">{goal()?.error ?? 'The last turn was cut off by an error.'}</p>
          <button type="button" class="lh-retry" disabled={busy()} onClick={() => void retry()} data-testid="lh-loop-retry">
            <Show when={busy()} fallback={<RotateCcw size={12} />}><Loader2 size={12} class="lh-spin" /></Show> Retry
          </button>
        </div>
      </Show>
      <h4 class="lh-sub-title">Timeline</h4>
      <LhTimeline entries={timeline()} empty="Events appear here as the loop runs." />
    </div>
  );
}
