/**
 * Tool group (WP-A8, Z-38): consecutive same-class tools merged under one
 * Execute / Explored / Changed head inside the fold. Collapsed by default
 * when settled, open while its segment runs; the member rows (and their
 * thinking neighbours, in order) mount only while open.
 */
import { Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { ChevronRight } from 'lucide-solid';
import { layout } from '../../../lib/layout';
import { toolCategory } from '../../../lib/chat/model';
import { createPresence } from '../../../lib/motion';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import type { WorkNode } from '../../../lib/session/turns';
import type { TimelineCtx } from '../context';
import { GROUP_LABEL, TOOL_ICON } from './toolIcons';
import { WorkItemView } from './WorkItem';
import { ByKey } from '../ByKey';

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Phone (§3.7, PH-27): "Ran 2 commands, read 1 file" — what the group did, in words. */
export function groupSummary(tools: ReadonlyArray<{ name: string }>): string {
  const n = { run: 0, read: 0, edit: 0, search: 0, other: 0 };
  for (const t of tools) {
    const c = toolCategory(t.name);
    if (c === 'run' || c === 'read' || c === 'edit' || c === 'search') n[c] += 1;
    else n.other += 1;
  }
  const parts = [
    n.run && `ran ${plural(n.run, 'command', 'commands')}`,
    n.read && `read ${plural(n.read, 'file', 'files')}`,
    n.edit && `edited ${plural(n.edit, 'file', 'files')}`,
    n.search && `searched ${plural(n.search, 'time', 'times')}`,
    n.other && `used ${plural(n.other, 'tool', 'tools')}`,
  ].filter(Boolean) as string[];
  const text = parts.join(', ');
  return text ? text[0]!.toUpperCase() + text.slice(1) : '';
}

export function ToolGroup(props: { node: Extract<WorkNode, { kind: 'group' }>; ctx: TimelineCtx; live: boolean }) {
  const open = () => expansionOpen(props.ctx.sessionId(), `grp:${props.node.key}`, props.live);
  const toggle = () => toggleExpansion(props.ctx.sessionId(), `grp:${props.node.key}`, props.live);
  const stepText = () => `${props.node.tools.length} ${props.node.tools.length === 1 ? 'step' : 'steps'}`;
  const anyError = () => props.node.tools.some((t) => t.state === 'error');
  const body = createPresence(open);
  const phone = () => !layout().desktop();
  const failed = () => props.node.tools.filter((t) => t.state === 'error').length;
  let root: HTMLDivElement | undefined;
  const onToggle = () => {
    const opening = !open();
    toggle();
    // Expanding scrolls the expanded block into view (PH-27).
    if (opening && phone()) requestAnimationFrame(() => root?.scrollIntoView({ block: 'nearest', behavior: 'smooth' }));
  };
  return (
    <div ref={root} class="tl-group" data-open={open() ? '' : undefined} data-kind={props.node.group} data-testid="tl-group">
      <button
        type="button"
        class="proc-head tl-group-head"
        aria-expanded={open()}
        onClick={onToggle}
        title={`${GROUP_LABEL[props.node.group]} · ${stepText()}`}
      >
        <span class="proc-icon">
          <Dynamic component={TOOL_ICON[props.node.group === 'execute' ? 'run' : props.node.group === 'explore' ? 'search' : 'edit']} size={15} />
        </span>
        <Show
          when={!phone()}
          fallback={
            <>
              <span class="proc-title ph-group-summary">{groupSummary(props.node.tools) || GROUP_LABEL[props.node.group]}</span>
              <Show when={failed() > 0 && !props.live}>
                <span class="ph-fail-pill" data-testid="tl-group-failed">
                  {failed()} failed
                </span>
              </Show>
            </>
          }
        >
          <span class="proc-title">{GROUP_LABEL[props.node.group]}</span>
          <span class="proc-detail" data-error={anyError() && !props.live ? '' : undefined}>
            {stepText()}{anyError() ? ` · ${props.node.tools.filter((t) => t.state === 'error').length} failed` : ''}
          </span>
        </Show>
        <ChevronRight size={15} class="proc-chev" />
      </button>
      <Show when={body.mounted()}>
        <div class="proc-body tl-group-body" data-leaving={body.leaving() ? '' : undefined}>
          <ByKey each={props.node.items} key={(it) => it.key}>{(item) => <WorkItemView item={item()} ctx={props.ctx} />}</ByKey>
        </div>
      </Show>
    </div>
  );
}
