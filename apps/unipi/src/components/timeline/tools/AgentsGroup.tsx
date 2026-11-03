/**
 * Parallel subagent runs ([ui-subagents]): consecutive `run_subagent` calls of
 * one turn stack under a single "Running 3 subagents" container — one card
 * each with its own live status — so their child steps never interleave as
 * loose rows. Open by default (the cards are the content); its head carries
 * the per-status counts.
 *
 * perf-app: a big fan-out (≥ `AGENTS_VIRTUAL_MIN` runs — a 200-child graph)
 * renders its cards in a bounded inner scroller, virtualized: only the cards
 * on screen (+ overscan) are in the DOM, measured as they open and close.
 */
import { createSignal, Index, Show } from 'solid-js';
import { useVirtual } from '../../../ui/virtual';
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

/** Runs from which the cards virtualize in their own scroller. */
export const AGENTS_VIRTUAL_MIN = 24;
/** A collapsed card's height (head + one live line + the 6 px gap) before it is measured. */
const AGENT_CARD_ESTIMATE = 64;

/** The cards of a big fan-out: an inner scroller with only the visible cards mounted. */
function VirtualAgentCards(props: { items: readonly WorkItem[]; ctx: TimelineCtx }) {
  const [scroller, setScroller] = createSignal<HTMLElement>();
  const virt = useVirtual({
    count: () => props.items.length,
    scroller,
    estimate: () => AGENT_CARD_ESTIMATE,
    key: (i) => props.items[i]?.key ?? i,
    overscan: 6,
  });
  return (
    <div class="tl-agents-scroll" ref={setScroller} data-testid="tl-agents-virtual">
      <div class="tl-agents-space" style={{ height: `${virt.getTotalSize()}px` }}>
        <Index each={virt.getVirtualItems()}>
          {(v) => {
            const item = () => props.items[v().index];
            // Remount a slot only when it shows another run; updates of the same run flow through.
            const key = () => item()?.key;
            return (
              <div
                class="tl-agents-vrow"
                data-index={v().index}
                ref={(el) => queueMicrotask(() => el.isConnected && virt.measureElement(el))}
                style={{ transform: `translateY(${v().start}px)` }}
              >
                <Show when={key()} keyed>
                  {(_key: string) => <Show when={item()}>{(it) => <WorkItemView item={it()} ctx={props.ctx} />}</Show>}
                </Show>
              </div>
            );
          }}
        </Index>
      </div>
    </div>
  );
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
          <Show
            when={props.node.items.length >= AGENTS_VIRTUAL_MIN}
            fallback={
              <ByKey each={props.node.items} key={(it: WorkItem) => it.key}>
                {(item) => <WorkItemView item={item()} ctx={props.ctx} />}
              </ByKey>
            }
          >
            <VirtualAgentCards items={props.node.items} ctx={props.ctx} />
          </Show>
        </div>
      </Show>
    </div>
  );
}
