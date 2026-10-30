/**
 * UNI-222: the chat's Progress sheet (header ⋯ → Progress, palette "Show
 * long-horizon progress"): the goal / ralph / swarm / graph run of this pi,
 * live. Graph and swarm draw a mermaid chart (nodes coloured by status,
 * dependency edges, waves as subgraphs — tap to open it full-screen) plus a
 * status list; ralph is a checklist with an iteration bar; goal is a % bar,
 * turn count and the latest estimate. The log under it is the same state's
 * transitions (docs/m5/PROTOCOL.md §7). Empty → EmptyState.
 *
 * UNI-282 (§7.2, Maka's AgentGraphPanel): graph/swarm steps expand to their
 * profile / time / tool calls / tokens, the committed summary, **Full result**
 * (`lh_graph output`, markdown) and **Open session** (`work_transcript` of the
 * step's subagent); results the supervisor's finish chose carry a "selected"
 * chip; **Stop graph** (confirm) while it runs; a run picker switches between
 * the current run and earlier graphs of this project (`lh_graph history`,
 * read-only).
 */
import { createEffect, createMemo, createSignal, For, Index, on, onCleanup, Show, type JSX } from 'solid-js';
import { Check, ChevronDown, ChevronRight, CircleDashed, FileText, History, Maximize2, MessageSquare, Network, Square, X as XIcon } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { EmptyState } from '../EmptyState';
import { MdBlock } from './MdBlock';
import { StatusBar } from './StatusBar';
import { openMermaidViewer } from './MermaidViewer';
import { renderMermaid } from '../../lib/markdown/mermaid';
import { renderBlocks } from '../../lib/markdown';
import type { Chat } from '../../lib/chat/store';
import { durationText, startedText } from '../../lib/chat/dream';
import {
  agoText,
  clockText,
  isSelected,
  ITEM_LABEL,
  MODE_LABEL,
  progressMermaid,
  RUN_LABEL,
  runPercent,
  shownRun,
  stepDurationMs,
  tokensText,
  type LhGraphOutput,
  type LhItem,
  type LhMode,
  type LhRun,
  type TranscriptItem,
  type WorkTranscript,
} from '../../lib/chat/progress';

/** Is there a run worth a dot on the menu row? */
export function progressActive(chat: Chat | undefined): boolean {
  return chat?.state.lhProgress?.current?.status === 'running';
}

/** Earlier graphs (`lh_graph history`): not asked yet / loading / the runs / unsupported or failed. */
type History = { state: 'idle' } | { state: 'loading' } | { state: 'ok'; runs: LhRun[] } | { state: 'error'; error: string; unsupported?: boolean };

export function ProgressSheet(props: { open: boolean; onClose: () => void; chat: Chat }) {
  const s = () => props.chat.state;
  // Pull once on open (an older hello may predate the run); pushes keep it live.
  createEffect(on(() => props.open && s().conn === 'live', (go) => go && props.chat.refreshProgress?.()));
  const [now, setNow] = createSignal(Date.now());
  createEffect(() => {
    if (!props.open) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(t));
  });
  // Stop graph (UNI-282): its own confirm sheet, like the Dream sheet's Stop.
  const [confirmStop, setConfirmStop] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [flash, setFlash] = createSignal<{ text: string; tone?: 'error' } | null>(null);
  // UNI-282: earlier graphs, asked for each time the sheet opens (lazily — never while closed).
  const [history, setHistory] = createSignal<History>({ state: 'idle' });
  const [picked, setPicked] = createSignal<number | null>(null);
  const [pickerOpen, setPickerOpen] = createSignal(false);
  createEffect(
    on(
      () => props.open && s().conn === 'live',
      (go) => {
        if (!go) {
          if (!props.open) (setPicked(null), setPickerOpen(false), setConfirmStop(false), setFlash(null));
          return;
        }
        if (!props.chat.graphHistory) return;
        setHistory({ state: 'loading' });
        void props.chat.graphHistory().then((r) => setHistory(Array.isArray(r) ? { state: 'ok', runs: r } : { state: 'error', error: r.error, unsupported: r.unsupported }));
      },
    ),
  );
  const pastRuns = () => {
    const h = history();
    return h.state === 'ok' ? h.runs : [];
  };
  const pickedRun = () => {
    const i = picked();
    return i === null ? undefined : pastRuns()[i];
  };
  const shown = createMemo(() => shownRun(s().lhProgress));
  /** UNI-274: the sticky working mode (footer "Goal mode") before the agent starts a run. */
  const stickyMode = (): LhMode | undefined => {
    const m = s().lhMode?.mode;
    return m === 'goal' || m === 'ralph' || m === 'swarm' || m === 'graph' ? m : undefined;
  };
  const title = () => {
    const run = pickedRun() ?? shown()?.run;
    return run ? `${MODE_LABEL[run.mode]} progress` : 'Progress';
  };
  const stopMode = () => (shown()?.run.mode === 'swarm' ? 'swarm' : 'graph');
  const stop = async () => {
    setBusy(true);
    setFlash(null);
    try {
      await props.chat.graphStop();
      setFlash({ text: `Stopping the ${stopMode()} — running steps are cancelled.` });
    } catch (error) {
      setFlash({ text: `Couldn’t stop the ${stopMode()}: ${error instanceof Error ? error.message : String(error)}`, tone: 'error' });
    } finally {
      setBusy(false);
      setConfirmStop(false);
    }
  };
  return (
    <>
      <BoardSheet open={props.open && !confirmStop()} onClose={props.onClose} label="Progress" title={title()} class="progress-sheet" testId="progress-sheet">
        <Show when={flash()}>
          {(f) => (
            <p class="progress-flash" data-tone={f().tone} role={f().tone === 'error' ? 'alert' : 'status'} data-testid="progress-flash">
              {f().text}
            </p>
          )}
        </Show>
        <Show when={pastRuns().length > 0}>
          <RunPicker
            runs={pastRuns()}
            picked={picked()}
            open={pickerOpen()}
            now={now()}
            hasCurrent={!!shown()}
            onToggle={() => setPickerOpen((v) => !v)}
            onPick={(i) => (setPicked(i), setPickerOpen(false))}
          />
        </Show>
        <Show
          when={pickedRun()}
          fallback={
            <Show
              when={shown()}
              fallback={
                <EmptyState
                  compact
                  testId="progress-empty"
                  icon={<Network size={22} />}
                  title={s().lhProgressSupported === false ? 'Progress needs a newer unipi' : stickyMode() ? `${MODE_LABEL[stickyMode()!]} mode is on` : 'No long-horizon run'}
                  body={
                    s().lhProgressSupported === false
                      ? 'Update unipi on the PC and restart pi to see goal, ralph, swarm and graph progress here.'
                      : stickyMode()
                        ? `Nothing to show until the agent starts a ${MODE_LABEL[stickyMode()!].toLowerCase()}. It shows here live.`
                        : 'Start one in this chat with /unipi:goal, /unipi:ralph, /unipi:swarm or /unipi:graph. Its progress shows here live.'
                  }
                />
              }
            >
              {(sh) => (
                <RunView
                  chat={props.chat}
                  run={sh().run}
                  live={sh().live}
                  idle={!s().lhProgress?.current}
                  stickyMode={stickyMode()}
                  now={now()}
                  log={s().lhProgress?.log ?? []}
                  updatedAt={s().lhProgress?.updatedAt ?? 0}
                  onStop={() => (setFlash(null), setConfirmStop(true))}
                />
              )}
            </Show>
          }
        >
          {(run) => <RunView chat={props.chat} run={run()} live={false} idle={false} past now={now()} log={[]} updatedAt={0} />}
        </Show>
      </BoardSheet>
      <BoardSheet
        open={props.open && confirmStop()}
        onClose={() => !busy() && setConfirmStop(false)}
        label={`Stop the ${stopMode()}?`}
        testId="graph-stop-confirm"
        footer={
          <div class="dialog-actions">
            <button type="button" class="btn btn-secondary" disabled={busy()} onClick={() => setConfirmStop(false)}>
              Cancel
            </button>
            <button type="button" class="btn btn-danger" disabled={busy()} data-testid="graph-stop-go" onClick={() => void stop()}>
              {busy() ? 'Stopping…' : 'Stop'}
            </button>
          </div>
        }
      >
        <p class="move-why mt-0">Running steps are cancelled. Finished steps keep their results, and the run stays in the history.</p>
      </BoardSheet>
    </>
  );
}

/** "Current" vs earlier graphs of this project (UNI-282, Maka's run selector). */
function RunPicker(props: { runs: LhRun[]; picked: number | null; open: boolean; now: number; hasCurrent: boolean; onToggle: () => void; onPick: (i: number | null) => void }) {
  const pickedRun = () => (props.picked === null ? undefined : props.runs[props.picked]);
  return (
    <div class="progress-runs" data-testid="progress-runs">
      <div class="progress-runs-strip">
        <button type="button" class="progress-run-chip" aria-pressed={props.picked === null} data-testid="run-current" onClick={() => props.onPick(null)}>
          {props.hasCurrent ? 'Current' : 'Now'}
        </button>
        <button type="button" class="progress-run-chip" aria-pressed={props.picked !== null} aria-expanded={props.open} data-testid="run-earlier" onClick={() => props.onToggle()}>
          <History size={13} />
          <span class="truncate">{pickedRun() ? pickedRun()!.title : `Earlier runs · ${props.runs.length}`}</span>
          <ChevronDown size={13} class="shrink-0" />
        </button>
      </div>
      <Show when={props.open}>
        <ul class="progress-run-list" data-testid="run-list">
          <For each={props.runs}>
            {(run, i) => (
              <li>
                <button type="button" class="progress-run-row" aria-pressed={props.picked === i()} onClick={() => props.onPick(i())}>
                  <StatusGlyph status={run.status === 'done' ? 'done' : run.status === 'failed' ? 'failed' : 'aborted'} />
                  <span class="min-w-0 flex-1 text-left">
                    <span class="progress-run-title">{run.title}</span>
                    <span class="progress-run-sub">
                      {MODE_LABEL[run.mode]}
                      {run.endedAt !== undefined ? ` · ${startedText(run.endedAt, props.now)}` : ''} · {run.counts.done + run.counts.failed}/{run.counts.total} settled
                    </span>
                  </span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}

function RunView(props: {
  chat: Chat;
  run: LhRun;
  live: boolean;
  idle: boolean;
  /** An earlier run from the picker: read-only, no log. */
  past?: boolean;
  stickyMode?: LhMode;
  now: number;
  log: Array<{ at: number; text: string; status?: string }>;
  updatedAt: number;
  onStop?: () => void;
}) {
  const r = () => props.run;
  const [allLog, setAllLog] = createSignal(false);
  const log = () => props.log.slice(allLog() ? -20 : -3).reverse();
  const ended = () => (r().endedAt !== undefined ? ` ended ${agoText(props.now - r().endedAt!)}` : '');
  return (
    <div class="progress-body" data-mode={r().mode} data-live={props.live ? '' : undefined} data-past={props.past ? '' : undefined}>
      <div class="progress-head">
        <span class="progress-status" data-status={r().status}>
          <Show when={props.live} fallback={<StatusGlyph status={r().status === 'done' ? 'done' : r().status === 'failed' ? 'failed' : 'queued'} />}>
            <span class="progress-pulse" aria-hidden="true" />
          </Show>
          {MODE_LABEL[r().mode]} · {RUN_LABEL[r().status]}
        </span>
        <span class="progress-updated">
          {!props.idle && !props.past && props.updatedAt ? `updated ${agoText(props.now - props.updatedAt)}` : ''}
        </span>
      </div>
      <Show when={props.past}>
        <p class="progress-idle" data-testid="progress-past">
          An earlier run{ended()} · read-only
        </p>
      </Show>
      <Show when={props.idle}>
        <p class="progress-idle" data-testid="progress-idle">
          {props.stickyMode ? `${MODE_LABEL[props.stickyMode]} mode is on, no run yet` : 'No long-horizon mode active'} · last run{ended()}
        </p>
      </Show>
      <h3 class="progress-title">{r().mode === 'goal' && r().goal ? r().goal!.objective : r().title}</h3>
      <Show when={r().reason && !props.live}>
        <p class="progress-reason">{r().reason}</p>
      </Show>

      <Show when={r().mode === 'goal'}>
        <GoalView run={r()} live={props.live} now={props.now} />
      </Show>
      <Show when={r().mode === 'ralph'}>
        <RalphView run={r()} live={props.live} />
      </Show>
      <Show when={r().mode === 'graph' || r().mode === 'swarm'}>
        <ItemsView chat={props.chat} run={r()} live={props.live} now={props.now} readOnly={props.past} onStop={props.onStop} />
      </Show>

      <Show when={props.log.length > 0}>
        <div class="progress-section">
          <span class="section-label">Recent</span>
          <Show when={props.log.length > 3}>
            <button type="button" class="progress-more" onClick={() => setAllLog((v) => !v)}>
              {allLog() ? 'Show less' : `Show all ${Math.min(20, props.log.length)}`}
            </button>
          </Show>
        </div>
        <ol class="progress-log" data-testid="progress-log">
          <For each={log()}>
            {(line, i) => (
              <li data-newest={i() === 0 ? '' : undefined} data-status={line.status}>
                <span class="progress-log-at">{clockText(line.at).slice(0, 5)}</span>
                <span class="progress-log-text">{line.text}</span>
              </li>
            )}
          </For>
        </ol>
      </Show>
    </div>
  );
}

function Bar(props: { value: number; max: number; tone?: string; label: string; testId?: string }) {
  const pct = () => (props.max > 0 ? Math.max(0, Math.min(100, (props.value / props.max) * 100)) : 0);
  return (
    <div class="progress-bar" role="progressbar" aria-label={props.label} aria-valuemin={0} aria-valuemax={props.max} aria-valuenow={props.value} data-testid={props.testId}>
      <div class="progress-bar-fill" data-tone={props.tone} style={{ width: `${pct()}%` }} />
    </div>
  );
}

function StatusGlyph(props: { status: LhItem['status'] }) {
  return (
    <span class="progress-glyph" data-status={props.status} aria-hidden="true">
      <Show when={props.status === 'done'} fallback={<Show when={props.status === 'failed' || props.status === 'aborted'} fallback={<CircleDashed size={14} />}><XIcon size={14} /></Show>}>
        <Check size={14} />
      </Show>
    </span>
  );
}

function CountsLine(props: { run: LhRun }) {
  const k = () => props.run.counts;
  return (
    <p class="progress-counts">
      <b>
        {k().done + k().failed}/{k().total}
      </b>
      <Show when={k().done}> · <span data-tone="done">{k().done} done</span></Show>
      <Show when={k().failed}> · <span data-tone="failed">{k().failed} failed</span></Show>
      <Show when={k().running}> · <span data-tone="running">{k().running} running</span></Show>
      <Show when={k().queued}> · <span data-tone="queued">{k().queued} waiting</span></Show>
    </p>
  );
}

function GoalView(props: { run: LhRun; live: boolean; now: number }) {
  const g = () => props.run.goal;
  return (
    <Show when={g()} fallback={<p class="progress-note">Goal details aren't available.</p>}>
      {(goal) => (
        <div class="progress-goal">
          <div class="progress-metric">
            <span class="progress-metric-value" data-testid="goal-percent">
              {goal().percent === undefined ? 'Not estimated yet' : `${goal().status === 'complete' ? '' : '~'}${goal().percent}%`}
            </span>
            <span class="progress-metric-label">
              turn {goal().turn} of {goal().maxTurns}
            </span>
          </div>
          <Bar value={goal().percent ?? 0} max={100} tone={props.run.status} label="Goal progress" testId="goal-bar" />
          <p class="progress-summary">
            {goal().summary || 'No estimate yet — /unipi:goal status asks for one.'}
            <Show when={goal().summary && goal().estimatedAt !== undefined}>
              <span class="progress-when"> · {agoText(props.now - goal().estimatedAt!)}</span>
            </Show>
          </p>
        </div>
      )}
    </Show>
  );
}

function RalphView(props: { run: LhRun; live: boolean }) {
  const r = () => props.run.ralph;
  return (
    <div class="progress-ralph">
      <Show when={r()}>
        {(ralph) => (
          <>
            <div class="progress-metric">
              <span class="progress-metric-value">
                {ralph().checked} of {ralph().total} checked · {runPercent(props.run)}%
              </span>
              <span class="progress-metric-label">
                iteration {ralph().iteration}
                {ralph().maxIterations > 0 ? ` of ${ralph().maxIterations}` : ''}
              </span>
            </div>
            <Bar value={ralph().checked} max={Math.max(1, ralph().total)} tone={props.run.status} label="Checklist" />
          </>
        )}
      </Show>
      <ul class="progress-checklist" data-testid="ralph-checklist">
        <For each={props.run.items}>
          {(item) => (
            <li data-status={item.status}>
              <Show when={item.status === 'running' && props.live} fallback={<StatusGlyph status={item.status} />}>
                <span class="progress-spinner" aria-hidden="true" />
              </Show>
              <span class="progress-check-text">{item.label}</span>
              <Show when={item.status === 'running'}>
                <span class="progress-chip">this iteration</span>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

type ItemsProps = { chat: Chat; run: LhRun; live: boolean; now: number; readOnly?: boolean; onStop?: () => void };

function ItemsView(props: ItemsProps) {
  const source = createMemo(() => (props.run.items.length ? progressMermaid(props.run) : ''));
  const [svg, setSvg] = createSignal<string | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  createEffect(
    on(source, (src) => {
      if (!src) return;
      let alive = true;
      void renderMermaid(src).then((res) => {
        if (!alive) return;
        if (res.ok) {
          setSvg(res.svg);
          setError(undefined);
        } else setError(res.error);
      });
      onCleanup(() => (alive = false));
    }),
  );
  const [openId, setOpenId] = createSignal<string | null>(null);
  // Another run (the picker): nothing stays expanded.
  // (A memo key: `on` re-fires whenever its source re-runs — every push — not only when the value changes.)
  const runKey = createMemo(() => `${props.run.graphId ?? ''}:${props.run.title}`);
  createEffect(on(runKey, () => setOpenId(null), { defer: true }));
  const settled = () => props.run.items.filter((i) => i.status === 'done' || i.status === 'failed' || i.status === 'aborted').length;
  /** A paused (parked) run, or a live one before its first update_agent_graph. */
  const emptyNote = () =>
    props.live || props.run.status === 'paused' ? 'Waiting for the plan — no items declared yet.' : 'This run ended before declaring any steps.';
  return (
    <div class="progress-items">
      <Show when={props.run.counts.total > 0}>
        <StatusBar counts={props.run.counts} live={props.live} label={`${MODE_LABEL[props.run.mode]} items settled`} testId="items-bar" />
        <CountsLine run={props.run} />
      </Show>
      <Show when={props.live && props.onStop && !props.readOnly}>
        <div class="progress-actions">
          <span class="progress-settled" data-testid="progress-settled">
            {settled()} of {props.run.items.length} settled
          </span>
          <button type="button" class="queue-action-btn" data-tone="danger" data-testid="graph-stop" onClick={() => props.onStop?.()}>
            <Square size={12} /> Stop {props.run.mode === 'swarm' ? 'swarm' : 'graph'}
          </button>
        </div>
      </Show>
      <Show when={props.run.items.length > 0} fallback={<p class="progress-note">{emptyNote()}</p>}>
        <div class="progress-chart-wrap">
          <Show when={svg()} fallback={<div class="progress-chart-loading">{error() ? `Couldn't draw the chart: ${error()}` : 'Drawing the chart…'}</div>}>
            <button
              type="button"
              class="progress-chart"
              aria-label="Open the chart full-screen"
              data-testid="progress-chart"
              // eslint-disable-next-line solid/no-innerhtml -- sanitised by renderMermaid (DOMPurify svg profile)
              innerHTML={svg()}
              onClick={() => openMermaidViewer({ svg: svg()!, source: source() })}
            />
            <span class="progress-chart-zoom" aria-hidden="true">
              <Maximize2 size={14} />
            </span>
          </Show>
        </div>
        <div class="section-label">{props.run.mode === 'graph' || props.run.mode === 'swarm' ? 'Steps' : 'Items'}</div>
        {/* Index, not For: every push replaces the snapshot's objects, and an open step's panels must survive it (step order is stable). */}
        <ul class="progress-list" data-testid="progress-items">
          <Index each={props.run.items}>
            {(item) => (
              <li data-status={item().status} data-selected={isSelected(props.run, item()) ? '' : undefined}>
                <button type="button" class="progress-item-row" aria-expanded={openId() === item().id} onClick={() => setOpenId(openId() === item().id ? null : item().id)}>
                  <Show when={item().status === 'running' && props.live} fallback={<StatusGlyph status={item().status} />}>
                    <span class="progress-spinner" aria-hidden="true" />
                  </Show>
                  <span class="min-w-0 flex-1 text-left">
                    <span class="progress-item-id">{item().id}</span>
                    <span class="progress-item-label">{item().label}</span>
                  </span>
                  <Show when={isSelected(props.run, item())}>
                    <span class="progress-chip progress-chip-selected" data-testid="selected-chip">
                      selected
                    </span>
                  </Show>
                  {/* The icon says done / running / queued; only what it can't say gets a word. */}
                  <Show when={item().status === 'failed' || item().status === 'aborted' || (item().attempts ?? 0) > 1}>
                    <span class="progress-item-status">
                      {item().status === 'failed' || item().status === 'aborted' ? ITEM_LABEL[item().status] : ''}
                      {(item().attempts ?? 0) > 1 ? ` ×${item().attempts}` : ''}
                    </span>
                  </Show>
                  <Show when={openId() === item().id} fallback={<ChevronRight size={16} class="shrink-0" />}>
                    <ChevronDown size={16} class="shrink-0" />
                  </Show>
                </button>
                <Show when={openId() === item().id}>
                  <StepDetail chat={props.chat} run={props.run} item={item()} live={props.live} now={props.now} />
                </Show>
              </li>
            )}
          </Index>
        </ul>
      </Show>
    </div>
  );
}

/** One step's facts: `worker · 3m 05s · 12 tool calls · 34k tokens`. */
function stepFacts(item: LhItem, now: number): string[] {
  const out: string[] = [];
  if (item.profile) out.push(item.profile);
  const ms = stepDurationMs(item, now);
  if (ms !== undefined) out.push(durationText(ms));
  if (item.toolCalls !== undefined) out.push(`${item.toolCalls} tool call${item.toolCalls === 1 ? '' : 's'}`);
  if (item.tokens !== undefined) out.push(`${tokensText(item.tokens)} tokens`);
  return out;
}

type Fetched<T> = { loading: true } | { value: T } | { error: string };

/** An expanded graph/swarm step: deps, facts, the clipped summary, Full result and Open session (UNI-282). */
function StepDetail(props: { chat: Chat; run: LhRun; item: LhItem; live: boolean; now: number }) {
  const it = () => props.item;
  const [panel, setPanel] = createSignal<'result' | 'session' | null>(null);
  const [result, setResult] = createSignal<Fetched<LhGraphOutput> | undefined>();
  const [session, setSession] = createSignal<Fetched<WorkTranscript> | undefined>();
  // Index reuses this row for another step/run: drop what was fetched for the old one.
  const stepKey = createMemo(() => `${props.run.graphId ?? ''}:${it().id}`);
  createEffect(on(stepKey, () => (setPanel(null), setResult(undefined), setSession(undefined)), { defer: true }));
  const loadResult = () => {
    setResult({ loading: true });
    void props.chat.graphOutput(it().id, props.run.graphId).then((r) => setResult('error' in r ? { error: r.error } : { value: r }));
  };
  const loadSession = () => {
    const id = it().operatorId;
    if (!id) return;
    setSession({ loading: true });
    void props.chat.workTranscript(id).then((r) => setSession('error' in r ? { error: r.error } : { value: r }));
  };
  const toggle = (which: 'result' | 'session') => {
    if (panel() === which) return setPanel(null);
    setPanel(which);
    // Re-fetch each time it opens: a running step's result/session keeps changing.
    if (which === 'result') loadResult();
    else loadSession();
  };
  const facts = () => stepFacts(it(), props.now);
  /** A queued/ready step has nothing to read yet. */
  const hasResult = () => it().status !== 'queued' && it().status !== 'ready';
  return (
    <div class="progress-item-detail" data-testid="step-detail">
      <Show when={props.run.mode === 'graph'}>
        <p>
          Wave {(it().wave ?? 0) + 1}
          {it().deps.length ? ` · needs ${it().deps.join(', ')}` : ' · no dependencies'}
        </p>
      </Show>
      <Show when={facts().length > 0}>
        <p class="progress-step-facts" data-testid="step-facts">
          {facts().join(' · ')}
        </p>
      </Show>
      <p class="progress-item-full">{it().label}</p>
      <Show when={it().summary}>
        <p class="progress-item-summary" data-status={it().status}>
          {it().summary}
        </p>
      </Show>
      <Show when={hasResult() || it().operatorId}>
        <div class="progress-step-actions">
          <Show when={hasResult()}>
            <button type="button" class="queue-action-btn" aria-pressed={panel() === 'result'} data-testid="step-result" onClick={() => toggle('result')}>
              <FileText size={12} /> Full result
            </button>
          </Show>
          <Show when={it().operatorId}>
            <button type="button" class="queue-action-btn" aria-pressed={panel() === 'session'} data-testid="step-session" onClick={() => toggle('session')}>
              <MessageSquare size={12} /> Open session
            </button>
          </Show>
        </div>
      </Show>
      <Show when={panel() === 'result' && result()}>
        {(r) => <ResultPanel fetched={r()} chat={props.chat} />}
      </Show>
      <Show when={panel() === 'session' && session()}>
        {(t) => <TranscriptPanel fetched={t()} chat={props.chat} running={it().status === 'running' && props.live} onRefresh={loadSession} />}
      </Show>
    </div>
  );
}

const fetchedValue = <T,>(f: Fetched<T>): T | undefined => ('value' in f ? f.value : undefined);
const fetchedError = <T,>(f: Fetched<T>): string | undefined => ('error' in f ? f.error : undefined);

/** Loading… / the error / the value — one fetched panel's frame. */
function FetchedPanel<T>(props: { fetched: Fetched<T>; loading: string; testId: string; children: (value: T) => JSX.Element }) {
  return (
    <div class="progress-step-panel" data-testid={props.testId}>
      <Show when={!('loading' in props.fetched)} fallback={<p class="progress-note">{props.loading}</p>}>
        <Show when={fetchedValue(props.fetched)} fallback={<p class="progress-step-error" role="alert">{fetchedError(props.fetched)}</p>}>
          {(v) => props.children(v())}
        </Show>
      </Show>
    </div>
  );
}

function ResultPanel(props: { fetched: Fetched<LhGraphOutput>; chat: Chat }) {
  return (
    <FetchedPanel fetched={props.fetched} loading="Loading the result…" testId="step-result-panel">
      {(out) => {
        const blocks = out.text?.trim() ? renderBlocks(out.text, false) : [];
        return (
          <>
            <Show when={out.failure}>
              <p class="progress-step-error">{out.failure}</p>
            </Show>
            <Show when={blocks.length > 0} fallback={<Show when={!out.failure}><p class="progress-note">No result text yet.</p></Show>}>
              <div class="progress-step-md chat-prose">
                <For each={blocks}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
              </div>
            </Show>
            <Show when={out.truncated}>
              <p class="progress-note">The result was clipped on the PC.</p>
            </Show>
          </>
        );
      }}
    </FetchedPanel>
  );
}

function TranscriptPanel(props: { fetched: Fetched<WorkTranscript>; chat: Chat; running: boolean; onRefresh: () => void }) {
  return (
    <FetchedPanel fetched={props.fetched} loading="Loading the session…" testId="step-session-panel">
      {(t) => {
        const report = t.report?.trim() ? renderBlocks(t.report, false) : [];
        return (
          <>
            <Show when={props.running}>
              <button type="button" class="progress-more" onClick={() => props.onRefresh()}>
                Refresh
              </button>
            </Show>
            <Show when={t.items.length > 0 || report.length > 0} fallback={<p class="progress-note">Nothing in this session yet.</p>}>
              <ol class="progress-transcript" data-testid="step-transcript">
                <For each={t.items}>{(item) => <TranscriptRow item={item} chat={props.chat} />}</For>
              </ol>
            </Show>
            <Show when={report.length > 0}>
              <div class="section-label">Report</div>
              <div class="progress-step-md chat-prose">
                <For each={report}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
              </div>
            </Show>
          </>
        );
      }}
    </FetchedPanel>
  );
}

function TranscriptRow(props: { item: TranscriptItem; chat: Chat }) {
  const i = props.item;
  if (i.kind === 'tool') return <ToolRow item={i} />;
  if (i.kind === 'task') {
    return (
      <li class="progress-tx-task" data-kind="task">
        <span class="progress-tx-kind">Task</span>
        <p>{i.text}</p>
      </li>
    );
  }
  return (
    <li class="progress-tx-text" data-kind="text">
      <For each={renderBlocks(i.text, false)}>{(html) => <MdBlock html={html} class="chat-block" chat={props.chat} />}</For>
    </li>
  );
}

/** A tool call: name + arg on one line; tap for its (clipped) output. */
function ToolRow(props: { item: Extract<TranscriptItem, { kind: 'tool' }> }) {
  const [open, setOpen] = createSignal(false);
  const i = () => props.item;
  return (
    <li class="progress-tx-tool" data-kind="tool" data-error={i().isError ? '' : undefined}>
      <button type="button" class="progress-tx-tool-row" aria-expanded={open()} onClick={() => setOpen((v) => !v)}>
        <Show when={i().running} fallback={<span class="progress-tx-glyph" aria-hidden="true">{i().isError ? '✗' : '✓'}</span>}>
          <span class="progress-spinner" aria-hidden="true" />
        </Show>
        <span class="progress-tx-name">{i().name}</span>
        <span class="progress-tx-arg">{i().arg}</span>
        <Show when={i().durationMs !== undefined}>
          <span class="progress-tx-dur">{i().durationMs! < 1000 ? `${Math.round(i().durationMs!)}ms` : durationText(i().durationMs!)}</span>
        </Show>
        <Show when={open()} fallback={<ChevronRight size={14} class="shrink-0" />}>
          <ChevronDown size={14} class="shrink-0" />
        </Show>
      </button>
      <Show when={open()}>
        <pre class="progress-tx-output" data-testid="tool-output">{i().output || '(no output)'}</pre>
      </Show>
    </li>
  );
}
