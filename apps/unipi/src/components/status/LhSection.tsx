/**
 * The status pane's long-horizon section (WP-A16, §3.10; reserved
 * `unipi.long-horizon/visualize`): the Progress sheet's content re-homed —
 * mode, goal, elapsed, iterations, graph counts, stop/resume and the run
 * history — fed by the LH sticky topics (`lib/status/lh.ts`). The sheet is
 * gone (§3.10: no sheet, no ModeChip popover body); the section renders only
 * while `unipi.long-horizon` is enabled (owner enablement gates it).
 */
import { createEffect, createMemo, createResource, createSignal, on, onCleanup, Show } from 'solid-js';
import { History, Pause, Play, Square } from 'lucide-solid';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { liveSource, parseExtensions } from '../session/modalData';
import { ModeIcon } from '../chat/ModeChip';
import { durationText } from './lhProgress';
import { LhGraph } from './LhGraph';
import {
  graphCountLine,
  graphSettled,
  lhModeCommand,
  lhSummary,
  LH_EXTENSION_ID,
  LH_PROGRESS_TOPIC,
  LH_STATE_TOPIC,
  parseLhProgress,
  parseLhState,
  type LhSummary,
} from '../../lib/status/lh';
import './status.css';

/** `unipi.long-horizon` present and enabled → the LH section renders. */
export function useLhEnabled(s: () => SessionHandle): () => boolean | undefined {
  const [enabled] = createResource(
    liveSource(s),
    async (state) => {
      if (state !== 'live') return undefined;
      const list = parseExtensions(await s().cmd<unknown>({ type: '_oino/extensions' }).catch(() => undefined));
      const lh = list.find((e) => e.id === LH_EXTENSION_ID);
      return !!lh && lh.enabled;
    },
  );
  return () => enabled.latest;
}

/** Now, ticking once a second while mounted (the elapsed clock). */
export function useClock(): () => number {
  const [now, setNow] = createSignal(Date.now());
  const t = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(t));
  return now;
}

const PHASE_WORD: Record<LhSummary['phase'], string> = { active: 'Running', selected: 'On', paused: 'Paused' };

export function LhSection(props: { hostId: string; sessionId: string; onToast?: (text: string) => void }) {
  const s = createMemo(() => sessions().ensure(props.hostId, props.sessionId));
  createEffect(
    on(s, (handle) => {
      const release = handle.acquire();
      handle.subscribeTopics([LH_STATE_TOPIC, LH_PROGRESS_TOPIC]);
      onCleanup(release);
    }),
  );
  const enabled = useLhEnabled(s);
  const state = () => parseLhState(s().state.topics[LH_STATE_TOPIC]?.data);
  const progress = () => parseLhProgress(s().state.topics[LH_PROGRESS_TOPIC]?.data);
  const summary = createMemo(() => (enabled() ? lhSummary(state(), progress()) : undefined));
  const now = useClock();
  const elapsed = () => {
    const run = s().state.run;
    return run.running && run.startedAt ? durationText(Math.max(0, now() - run.startedAt)) : undefined;
  };

  const [armed, setArmed] = createSignal(false);
  const send = (cmd: string, done: string) => {
    void s()
      .cmd({ type: 'prompt', message: cmd })
      .then(() => props.onToast?.(done))
      .catch(() => props.onToast?.('The command did not go through.'));
  };
  const stop = () => {
    const m = summary();
    if (!m || m.phase !== 'active') return;
    if (!armed()) {
      setArmed(true);
      setTimeout(() => setArmed(false), 2500);
      return;
    }
    setArmed(false);
    send(lhModeCommand(m.mode, 'stop'), `${m.label} stopped`);
  };

  return (
    <Show when={enabled()}>
      <section class="status-section" data-testid="lh-section" aria-label="Long-horizon">
        <div class="status-section-head">
          <span class="status-section-title">
            <ModeIcon mode={summary()?.mode ?? 'none'} size={13} /> Long-horizon
          </span>
          <Show when={summary()}>
            {(m) => (
              <span class="status-chip" data-phase={m().phase} data-testid="lh-phase">
                {PHASE_WORD[m().phase]}
              </span>
            )}
          </Show>
        </div>
        <Show when={summary()} fallback={<p class="status-empty">No long-horizon run. Start one with /goal, /ralph, /swarm or /graph.</p>}>
          {(m) => (
            <div class="lh-body">
              <p class="lh-mode-line">
                <span class="lh-mode-name">{m().label}</span>
                <Show when={m().goal}>{(g) => <span class="lh-mode-objective truncate" title={g().objective}>{g().objective}</span>}</Show>
                <Show when={m().graph?.task}>{(t) => <span class="lh-mode-objective truncate" title={t()}>{t()}</span>}</Show>
                <Show when={m().ralph}>{(r) => <span class="lh-mode-objective truncate">loop “{r().name}”</span>}</Show>
              </p>

              <Show when={elapsed() && m().phase === 'active'}>
                <p class="lh-line" data-testid="lh-elapsed">Working for {elapsed()}</p>
              </Show>

              <Show when={m().goal}>
                {(g) => (
                  <div class="lh-metric" data-testid="lh-goal">
                    <div class="lh-metric-row">
                      <span>Turn {g().turn}{g().maxTurns > 0 ? ` of ${g().maxTurns}` : ''}</span>
                      <span class="status-dim">{g().status}</span>
                    </div>
                    <div class="status-bar" role="progressbar" aria-label="Goal turns" aria-valuemin={0} aria-valuemax={g().maxTurns || undefined} aria-valuenow={g().turn}>
                      <span style={{ width: `${g().maxTurns > 0 ? Math.min(100, (g().turn / g().maxTurns) * 100) : 0}%` }} data-mode="goal" />
                    </div>
                  </div>
                )}
              </Show>

              <Show when={m().ralph}>
                {(r) => (
                  <div class="lh-metric" data-testid="lh-ralph">
                    <div class="lh-metric-row">
                      <span>Iteration {r().iteration}</span>
                      <span class="status-dim">{r().checked}/{r().total} checked</span>
                    </div>
                    <Show when={r().total > 0}>
                      <div class="status-bar" role="progressbar" aria-label="Checklist" aria-valuemin={0} aria-valuemax={r().total} aria-valuenow={r().checked}>
                        <span style={{ width: `${Math.min(100, (r().checked / r().total) * 100)}%` }} data-mode="ralph" />
                      </div>
                    </Show>
                  </div>
                )}
              </Show>

              <Show when={m().graph}>
                {(g) => (
                  <div class="lh-metric" data-testid="lh-graph">
                    <div class="lh-metric-row">
                      <span class="status-dim capitalize">{g().mode}</span>
                      <span>{graphCountLine(g().counts)}</span>
                    </div>
                    <Show when={graphSettled(g().counts).total > 0}>
                      <div class="status-bar" role="progressbar" aria-label="Graph progress" aria-valuemin={0} aria-valuemax={graphSettled(g().counts).total} aria-valuenow={graphSettled(g().counts).settled}>
                        <span style={{ width: `${m().percent ?? 0}%` }} data-mode={g().mode} />
                        <Show when={graphSettled(g().counts).failed > 0}>
                          <span class="status-bar-fail" style={{ width: `${(graphSettled(g().counts).failed / graphSettled(g().counts).total) * 100}%` }} data-testid="lh-graph-failed" />
                        </Show>
                      </div>
                    </Show>
                    {/* P1-04: the DAG and one row per node, each opening its worker session. */}
                    <LhGraph s={s()} counts={JSON.stringify(g().counts)} onToast={props.onToast} />
                  </div>
                )}
              </Show>

              <div class="status-actions">
                <Show
                  when={m().phase === 'active'}
                  fallback={
                    <Show when={m().phase === 'paused'}>
                      <button type="button" class="status-action" data-testid="lh-resume" onClick={() => send(lhModeCommand(m().mode, 'resume'), `${m().label} resumed`)}>
                        <Play size={12} /> Resume
                      </button>
                    </Show>
                  }
                >
                  <button type="button" class="status-action" data-tone={armed() ? 'danger' : undefined} data-testid="lh-stop" onClick={stop}>
                    <Show when={armed()} fallback={<Square size={12} />}>
                      <Pause size={12} />
                    </Show>
                    {armed() ? 'Tap again to stop' : 'Stop'}
                  </button>
                </Show>
                <Show when={m().graph && (m().mode === 'graph' || m().mode === 'swarm')}>
                  <button type="button" class="status-action" data-testid="lh-history" title="Earlier runs print in the chat" onClick={() => send(lhModeCommand(m().mode, 'history'), 'History printed in the chat')}>
                    <History size={12} /> History
                  </button>
                </Show>
              </div>
            </div>
          )}
        </Show>
      </section>
    </Show>
  );
}
