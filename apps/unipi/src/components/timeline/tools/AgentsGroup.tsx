/**
 * Parallel subagent runs ([ui-subagents]): consecutive `run_subagent` calls of
 * one turn stack under a single "Running 3 subagents" container — one card
 * each with its own live status — so their child steps never interleave as
 * loose rows. Open by default (the cards are the content); its head carries
 * the per-status counts.
 */
import { Show } from 'solid-js';
import { Bot, ChevronRight } from 'lucide-solid';
import { createPresence } from '../../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import { agentsSummary } from '../../../lib/session/agentRuns';
import type { WorkItem, WorkNode } from '../../../lib/session/turns';
import type { TimelineCtx } from '../context';
import { ByKey } from '../ByKey';
import { WorkItemView } from './WorkItem';

/** "Running 3 subagents" / "Ran 3 subagents". */
export function agentsTitle(runs: readonly { status: string }[]): string {
  const n = runs.length;
  const noun = n === 1 ? 'subagent' : 'subagents';
  return runs.some((r) => r.status === 'running') ? `Running ${n} ${noun}` : `Ran ${n} ${noun}`;
}

export function AgentsGroup(props: { node: Extract<WorkNode, { kind: 'agents' }>; ctx: TimelineCtx }) {
  const key = () => `agents:${props.node.key}`;
  const open = () => expansionOpen(props.ctx.sessionId(), key(), true);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), key(), true);
  const running = () => props.node.runs.some((r) => r.status === 'running');
  // "3 subagents · 2 running · 1 done" without the leading count the title already says.
  const detail = () => agentsSummary(props.node.runs).split(' · ').slice(1).join(' · ');
  const failed = () => props.node.runs.some((r) => r.status === 'failed');
  const body = createPresence(open);
  return (
    <div class="tl-agents" data-open={open() ? '' : undefined} data-live={running() ? '' : undefined} data-testid="tl-agents">
      <button type="button" class="proc-head tl-agents-head" aria-expanded={open()} onClick={toggle} title={`${agentsTitle(props.node.runs)} · ${detail()}`}>
        <span class="proc-icon">
          <Bot size={15} />
        </span>
        <span class="proc-title" classList={{ shimmer: running() }} data-testid="tl-agents-title">
          {agentsTitle(props.node.runs)}
        </span>
        <Show when={detail()}>
          <span class="proc-detail" data-error={failed() && !running() ? '' : undefined} data-testid="tl-agents-detail">
            {detail()}
          </span>
        </Show>
        <ChevronRight size={15} class="proc-chev" />
      </button>
      <Show when={body.mounted()}>
        <div class="tl-agents-body" data-leaving={body.leaving() ? '' : undefined}>
          <ByKey each={props.node.items} key={(it: WorkItem) => it.key}>
            {(item) => <WorkItemView item={item()} ctx={props.ctx} />}
          </ByKey>
        </div>
      </Show>
    </div>
  );
}
