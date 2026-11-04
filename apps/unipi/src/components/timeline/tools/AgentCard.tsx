/**
 * Subagent run card ([ui-subagents]): one card per `run_subagent` call — robot,
 * title, profile chip, status, elapsed and "12 tools · 3 files" — instead of
 * one top-level row per child tool. Collapsed it shows a single line: the
 * child's live activity while running (DeepSeek Harness `runningDetail`:
 * the latest running tool's argument, whitespace-collapsed and clipped),
 * the report excerpt when done, the error when failed. Expanded, the child's
 * steps render as an indented mini-timeline with its own scroll (grouped like
 * the parent's tools; the last 200 rows, earlier ones behind "Show earlier"),
 * and a child's own `run_subagent` steps nest as cards up to `AGENT_MAX_DEPTH`.
 *
 * The card reads only its `run` accessor: a child progress update reaches the
 * one card through `ByKey`'s per-key memo, not the timeline.
 */
import { createMemo, createSignal, For, Match, onCleanup, onMount, Show, Switch } from 'solid-js';
import { Ban, Bot, Check, ChevronRight, LoaderCircle, Moon, PanelRightOpen, X } from 'lucide-solid';
import { formatDuration } from '../../../lib/chat/model';
import { useClock } from '../../../lib/clock';
import { createPresence } from '../../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import { groupWork, type ToolRun, type WorkItem, type WorkNode } from '../../../lib/session/turns';
import { isAgentTool, liveLineOf, nestedRunOf, oneLine, runCounters, stepDataOf, type AgentRun, type AgentStatus } from '../../../lib/session/agentRuns';
import type { SessionEntry } from '../../../lib/session/types';
import type { TimelineCtx } from '../context';
import { openSubagentTab } from '../../panes/tabs/sessionTabs';
import { ByKey } from '../ByKey';
import { ToolRow } from './ToolRow';
import { ToolGroup } from './ToolGroup';

/** Child step rows a card renders at once (older ones behind "Show earlier"). */
export const AGENT_STEP_CAP = 200;
/** Nested run cards below this depth; deeper runs are a row that opens the transcript. */
export const AGENT_MAX_DEPTH = 3;

const STATUS_LABEL: Record<AgentStatus, string> = { running: 'Running', done: 'Done', failed: 'Failed', cancelled: 'Cancelled', background: 'In background' };

export function AgentStatusIcon(props: { status: AgentStatus }) {
  return (
    <span class="tl-agent-status" data-status={props.status} title={STATUS_LABEL[props.status]} aria-label={STATUS_LABEL[props.status]}>
      <Switch>
        <Match when={props.status === 'running'}>
          <LoaderCircle size={14} class="tl-agent-spin" />
        </Match>
        <Match when={props.status === 'done'}>
          <Check size={14} />
        </Match>
        <Match when={props.status === 'failed'}>
          <X size={14} />
        </Match>
        <Match when={props.status === 'cancelled'}>
          <Ban size={14} />
        </Match>
        <Match when={props.status === 'background'}>
          <Moon size={14} />
        </Match>
      </Switch>
    </span>
  );
}


/** A step entry as a mini-timeline work item (tool row, nested run, or the child's note). */
function stepItem(e: SessionEntry, depth: number): WorkItem {
  const d = stepDataOf(e);
  if (d.kind === 'text') return { kind: 'step', key: e.id, entry: e };
  const name = d.name ?? 'tool';
  if (isAgentTool(name)) return { kind: 'agent', key: e.id, run: nestedRunOf(e, depth + 1) };
  const args = d.args && typeof d.args === 'object' ? d.args : d.arg ? { arg: d.arg } : {};
  const tool: ToolRun = { id: e.id, name, args, state: d.isError ? 'error' : 'ok' };
  if (d.output !== undefined) tool.output = d.output;
  if (typeof d.durationMs === 'number') tool.durationMs = d.durationMs;
  return { kind: 'tool', key: e.id, tool };
}

/** The work nodes of a run's visible steps (consecutive reads / greps fold like the parent's). */
export function agentStepNodes(steps: readonly SessionEntry[], depth: number): WorkNode[] {
  return groupWork(steps.map((e) => stepItem(e, depth)));
}

/** `provider/model-id` → `model-id` (the chip's short form). */
const shortModel = (m: string | undefined): string | undefined => (m ? (m.split('/').pop() ?? m) : undefined);

export function AgentCard(props: {
  run: AgentRun;
  ctx: TimelineCtx;
  itemKey: string;
  /** Opens the transcript this card belongs to (nested cards open their root child's). */
  openTranscript?: () => void;
}) {
  const run = () => props.run;
  const key = () => `agent:${props.itemKey}`;
  const open = () => expansionOpen(props.ctx.sessionId(), key(), false);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), key(), false);
  const running = () => run().status === 'running';
  const now = useClock(() => run().status === 'running' || run().status === 'background');
  const elapsed = () => {
    const r = run();
    if (r.status === 'running' || r.status === 'background') return r.startedAt !== undefined ? formatDuration(Math.max(0, now() - r.startedAt)) : undefined;
    return r.durationMs !== undefined ? formatDuration(r.durationMs) : undefined;
  };
  const chip = () => run().profileLabel ?? (run().profile ? `${run().profile}` : undefined);
  const counters = () => runCounters(run());
  /** False when the sub line would hold nothing but the status icon (no chip/elapsed/counters). */
  const subHasMore = () => !!chip() || !!elapsed() || !!counters();
  const canOpen = () => !!run().agentId || !!props.openTranscript;
  const openTranscript = (ev?: MouseEvent) => {
    ev?.stopPropagation();
    const r = run();
    if (r.agentId) {
      openSubagentTab(
        { hostId: props.ctx.hostId, sessionId: props.ctx.sessionId(), cwd: props.ctx.cwd() },
        { agentId: r.agentId, name: r.title, ...(r.profile ? { profile: r.profile } : {}) },
      );
    } else props.openTranscript?.();
  };
  const line = createMemo(() => {
    const r = run();
    if (r.status === 'running') {
      const l = liveLineOf(r);
      return l ? { kind: 'live' as const, name: l.name, text: l.text } : { kind: 'live' as const, text: 'Starting…' };
    }
    if (r.status === 'failed' && r.error) return { kind: 'error' as const, text: oneLine(r.error, 240) };
    if (r.status === 'background') return { kind: 'muted' as const, text: 'Working in the background' };
    if (r.report) return { kind: 'report' as const, text: oneLine(r.report, 240) };
    if (r.status === 'cancelled') return { kind: 'muted' as const, text: 'Cancelled' };
    return undefined;
  });
  const deep = () => run().depth >= AGENT_MAX_DEPTH;
  const fullTitle = () => [run().title, chip(), STATUS_LABEL[run().status], elapsed(), counters()].filter(Boolean).join(' · ');
  const body = createPresence(() => open() && !deep());
  return (
    <div class="tl-agent" data-status={run().status} data-open={open() ? '' : undefined} data-depth={run().depth} data-testid="tl-agent">
      <div class="tl-agent-head">
        <button
          type="button"
          class="tl-agent-toggle"
          aria-expanded={deep() ? undefined : open()}
          onClick={() => (deep() ? openTranscript() : toggle())}
          title={fullTitle()}
          data-testid="tl-agent-toggle"
        >
          <span class="tl-agent-icon">
            <Bot size={15} />
          </span>
          {/* Two lines: the title gets the whole first line; profile/model + status/time/counters
              sit under it (one line truncated the title to a few words). */}
          <span class="tl-agent-main">
          <Show
            when={subHasMore()}
            fallback={
              /* Nothing else on the sub line: the status icon sits inline right
                after the title instead of alone on its own row (owner-fix-5). */
              <span class="tl-agent-titleline">
                <span class="tl-agent-title" classList={{ shimmer: running() }}>
                  {run().title}
                </span>
                <AgentStatusIcon status={run().status} />
              </span>
            }
          >
          <span class="tl-agent-title" classList={{ shimmer: running() }}>
            {run().title}
          </span>
          <span class="tl-agent-sub">
          <Show when={chip()}>
            <span class="tl-agent-chip" title={run().model ? `${chip()} · ${run().model}` : chip()} data-testid="tl-agent-chip">
              {chip()}
              <Show when={shortModel(run().model)}>
                <span class="tl-agent-model">{shortModel(run().model)}</span>
              </Show>
            </span>
          </Show>
          <span class="tl-agent-meta">
            <AgentStatusIcon status={run().status} />
            <Show when={elapsed()}>
              <span class="tl-agent-elapsed">{elapsed()}</span>
            </Show>
            <Show when={counters()}>
              <span class="tl-agent-counters" data-testid="tl-agent-counters">
                {counters()}
              </span>
            </Show>
          </span>
          </span>
          </Show>
          </span>
          <Show when={!deep()}>
            <ChevronRight size={15} class="proc-chev" />
          </Show>
        </button>
        <Show when={canOpen()}>
          <button type="button" class="tl-agent-open" onClick={openTranscript} title="Open the subagent transcript" aria-label="Open transcript" data-testid="tl-agent-open">
            <PanelRightOpen size={14} />
          </button>
        </Show>
      </div>
      <Show when={!open() && line()}>
        {(l) => (
          <div class="tl-agent-line" data-kind={l().kind} data-testid="tl-agent-line">
            <Show when={l().kind === 'live' && (l() as { name?: string }).name}>
              <span class="tl-agent-line-tool">{(l() as { name?: string }).name}</span>
            </Show>
            <span class="tl-agent-line-text">{l().text}</span>
          </div>
        )}
      </Show>
      <Show when={body.mounted()}>
        <AgentSteps run={run()} ctx={props.ctx} leaving={body.leaving()} openTranscript={canOpen() ? () => openTranscript() : undefined} />
      </Show>
    </div>
  );
}

/** The expanded mini-timeline: the last `AGENT_STEP_CAP` steps, "Show earlier" for the rest. */
function AgentSteps(props: { run: AgentRun; ctx: TimelineCtx; leaving: boolean; openTranscript?: () => void }) {
  const [limit, setLimit] = createSignal(AGENT_STEP_CAP);
  const hidden = () => Math.max(0, props.run.steps.length - limit());
  const visible = createMemo(() => props.run.steps.slice(hidden()));
  const nodes = createMemo(() => agentStepNodes(visible(), props.run.depth));
  const toolSteps = () => props.run.steps.filter((s) => stepDataOf(s).kind !== 'text').length;
  // Steps older than the loaded page (or written before the card existed) live in the child's transcript.
  const missing = () => hidden() === 0 && props.run.toolCalls > toolSteps();
  // Opens at the newest step and follows new output while the user is at the bottom
  // (scrolling up detaches; "Show earlier" keeps the reader's place).
  let el: HTMLDivElement | undefined;
  let stuck = true;
  let restoreFromBottom: number | undefined;
  const toBottom = () => el && (el.scrollTop = el.scrollHeight);
  onMount(() => {
    if (!el) return;
    toBottom();
    requestAnimationFrame(toBottom);
    const mo = new MutationObserver(() => {
      if (!el) return;
      if (restoreFromBottom !== undefined) {
        el.scrollTop = el.scrollHeight - el.clientHeight - restoreFromBottom;
        restoreFromBottom = undefined;
      } else if (stuck) toBottom();
    });
    mo.observe(el, { childList: true, subtree: true, characterData: true });
    const ro = new ResizeObserver(() => stuck && toBottom());
    ro.observe(el);
    onCleanup(() => (mo.disconnect(), ro.disconnect()));
  });
  const onScroll = () => {
    if (el) stuck = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };
  const showEarlier = () => {
    if (el) restoreFromBottom = el.scrollHeight - el.clientHeight - el.scrollTop;
    stuck = false;
    setLimit((n) => n + AGENT_STEP_CAP);
  };
  return (
    <div class="tl-agent-body" ref={el} onScroll={onScroll} data-leaving={props.leaving ? '' : undefined} data-testid="tl-agent-body">
      <Show when={hidden() > 0}>
        <button type="button" class="tl-agent-earlier" onClick={showEarlier} data-testid="tl-agent-earlier">
          Show {Math.min(hidden(), AGENT_STEP_CAP)} earlier {hidden() === 1 ? 'step' : 'steps'}
          <Show when={hidden() > AGENT_STEP_CAP}> (of {hidden()})</Show>
        </button>
      </Show>
      <Show when={missing() && props.openTranscript}>
        <button type="button" class="tl-agent-earlier" onClick={() => props.openTranscript?.()}>
          Earlier steps are in the transcript
        </button>
      </Show>
      <Show when={nodes().length > 0} fallback={<div class="tl-agent-empty">{props.run.status === 'running' ? 'No steps yet' : 'No steps recorded here'}</div>}>
        <ByKey each={nodes()} key={(n) => `${n.kind}:${n.key}`}>
          {(node) => <AgentStepNode node={node()} ctx={props.ctx} openTranscript={props.openTranscript} />}
        </ByKey>
      </Show>
    </div>
  );
}

function AgentStepNode(props: { node: WorkNode; ctx: TimelineCtx; openTranscript?: () => void }) {
  return (
    <Switch>
      <Match when={props.node.kind === 'group' && (props.node as Extract<WorkNode, { kind: 'group' }>)}>
        {(n) => <ToolGroup node={n()} ctx={props.ctx} live={false} />}
      </Match>
      <Match when={props.node.kind === 'tool' && (props.node as Extract<WorkNode, { kind: 'tool' }>)}>
        {(n) => <ToolRow tool={n().tool} ctx={props.ctx} itemKey={`agent-step:${n().key}`} />}
      </Match>
      <Match when={props.node.kind === 'agent' && (props.node as Extract<WorkNode, { kind: 'agent' }>)}>
        {(n) => <AgentCard run={n().run} ctx={props.ctx} itemKey={n().key} openTranscript={props.openTranscript} />}
      </Match>
      <Match when={props.node.kind === 'agents' && (props.node as Extract<WorkNode, { kind: 'agents' }>)}>
        {(n) => (
          <For each={n().runs}>
            {(r, i) => <AgentCard run={r} ctx={props.ctx} itemKey={`${n().key}:${i()}`} openTranscript={props.openTranscript} />}
          </For>
        )}
      </Match>
      <Match when={props.node.kind === 'step' && (props.node as Extract<WorkNode, { kind: 'step' }>)}>
        {(n) => <div class="tl-agent-note">{oneLine(stepDataOf(n().entry).text ?? '', 400)}</div>}
      </Match>
    </Switch>
  );
}
