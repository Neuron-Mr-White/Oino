/**
 * One work item inside a fold or group (WP-A8): dispatches to the tool /
 * thinking / prose / step / card / compaction / error renderers. Split out
 * of `SegmentFold` so groups nest member rows through the same path.
 */
import { Match, Switch } from 'solid-js';
import type { TimelineCtx } from '../context';
import type { WorkItem, WorkNode } from '../../../lib/session/turns';
import { Prose } from '../Prose';
import { Compaction } from '../Compaction';
import { ErrorRow } from '../ErrorRow';
import { sessions } from '../../../lib/session/store';
import { modelOrigin } from '../../../lib/model/displayName';
import { ToolRow } from './ToolRow';
import { ThinkingRow } from './ThinkingRow';
import { StepRow } from './StepRow';
import { CardRow } from '../cards/CardRow';
import { ToolGroup } from './ToolGroup';
import { AgentCard } from './AgentCard';
import { AgentsGroup } from './AgentsGroup';
import { ByKey } from '../ByKey';

/** `item` narrowed to one `WorkItem` kind for a `<Match>` `when`. */
const ofKind = <K extends WorkItem['kind']>(item: WorkItem, kind: K): Extract<WorkItem, { kind: K }> | false =>
  item.kind === kind ? (item as Extract<WorkItem, { kind: K }>) : false;

/** "Provider · model-id" of the session the error row is in (D-24). */
function failedModel(ctx: TimelineCtx): string | undefined {
  const m = sessions().peek(ctx.hostId, ctx.sessionId())?.state.model;
  return m?.id ? modelOrigin(m) : undefined;
}

export function WorkItemView(props: { item: WorkItem; ctx: TimelineCtx }) {
  const item = () => props.item;
  return (
    <Switch>
      <Match when={ofKind(item(), 'thinking')}>{(i) => <ThinkingRow item={i()} ctx={props.ctx} />}</Match>
      <Match when={ofKind(item(), 'tool')}>{(i) => <ToolRow tool={i().tool} ctx={props.ctx} itemKey={i().key} />}</Match>
      <Match when={ofKind(item(), 'prose')}>{(i) => <Prose text={i().text} live={i().live} entryKey={i().key} chat={props.ctx.refs} width={props.ctx.columnWidth} />}</Match>
      <Match when={ofKind(item(), 'agent')}>{(i) => <AgentCard run={i().run} ctx={props.ctx} itemKey={i().key} />}</Match>
      <Match when={ofKind(item(), 'step')}>{(i) => <StepRow entry={i().entry} ctx={props.ctx} />}</Match>
      <Match when={ofKind(item(), 'card')}>{(i) => <CardRow entry={i().entry} ctx={props.ctx} />}</Match>
      <Match when={ofKind(item(), 'compaction')}>{(i) => <Compaction entry={i().entry} sessionId={props.ctx.sessionId} />}</Match>
      <Match when={ofKind(item(), 'error')}>{(i) => <ErrorRow text={i().text} model={failedModel(props.ctx)} />}</Match>
    </Switch>
  );
}

/** Work nodes (groups render their head; `live` groups default open). */
export function WorkNodes(props: { nodes: readonly WorkNode[]; ctx: TimelineCtx; live?: boolean }) {
  return (
    <ByKey each={props.nodes} key={(n) => `${n.kind === 'group' ? 'g' : n.kind === 'agents' ? 'a' : 'i'}:${n.key}`}>
      {(node) => {
        const n = node();
        return n.kind === 'group' ? (
          <ToolGroup node={node() as Extract<WorkNode, { kind: 'group' }>} ctx={props.ctx} live={props.live ?? false} />
        ) : n.kind === 'agents' ? (
          <AgentsGroup node={node() as Extract<WorkNode, { kind: 'agents' }>} ctx={props.ctx} />
        ) : (
          <WorkItemView item={node() as WorkItem} ctx={props.ctx} />
        );
      }}
    </ByKey>
  );
}
