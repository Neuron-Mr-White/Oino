/**
 * Tool group (WP-A8, Z-38): consecutive same-class tools merged under one
 * Execute / Explored / Changed head inside the fold. Collapsed by default
 * when settled, open while its segment runs; the member rows (and their
 * thinking neighbours, in order) mount only while open.
 */
import { For, Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { ChevronRight } from 'lucide-solid';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import type { WorkNode } from '../../../lib/session/turns';
import type { TimelineCtx } from '../context';
import { GROUP_LABEL, TOOL_ICON } from './toolIcons';
import { WorkItemView } from './WorkItem';

export function ToolGroup(props: { node: Extract<WorkNode, { kind: 'group' }>; ctx: TimelineCtx; live: boolean }) {
  const open = () => expansionOpen(props.ctx.sessionId(), `grp:${props.node.key}`, props.live);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), `grp:${props.node.key}`, props.live);
  return (
    <div class="tl-group" data-open={open() ? '' : undefined} data-kind={props.node.group} data-testid="tl-group">
      <button type="button" class="proc-head tl-group-head" aria-expanded={open()} onClick={toggle}>
        <span class="proc-icon">
          <Dynamic component={TOOL_ICON[props.node.group === 'execute' ? 'run' : props.node.group === 'explore' ? 'search' : 'edit']} size={15} />
        </span>
        <span class="proc-title">{GROUP_LABEL[props.node.group]}</span>
        <span class="proc-detail">
          {props.node.tools.length} {props.node.tools.length === 1 ? 'step' : 'steps'}
        </span>
        <ChevronRight size={15} class="proc-chev" />
      </button>
      <Show when={open()}>
        <div class="proc-body tl-group-body">
          <For each={props.node.items}>{(item) => <WorkItemView item={item} ctx={props.ctx} />}</For>
        </div>
      </Show>
    </div>
  );
}
