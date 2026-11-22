import { For, type Accessor } from 'solid-js';
import type { PlacedEdge } from '../fleet';

/**
 * Every edge of the layout, drawn once. A `base` path is the line in the state-neutral track colour; the
 * `flow` path over it carries the child's state (green flowing, amber, red dashed, grey dotted).
 * Hovering a node highlights its ancestor path (`hl`) and fades the rest.
 */
export function Edges(props: { ids: Accessor<string[]>; byId: Accessor<Map<string, PlacedEdge>>; width: number; height: number; hl: Accessor<ReadonlySet<string> | undefined>; animated: Accessor<ReadonlySet<string>> }) {
  return <svg class="sac-edges" width={props.width} height={props.height} data-hover={props.hl() ? '' : undefined} aria-hidden="true">
    <For each={props.ids()}>{(id) => {
      const e = () => props.byId().get(id);
      return <path d={e()?.d} data-kind={e()?.kind} data-status={e()?.status} data-dim={e()?.dimmed ? '' : undefined} data-hl={props.hl()?.has(e()?.to ?? '') ? '' : undefined} data-anim={e()?.status === 'working' && props.animated().has(e()?.to ?? '') ? '' : undefined} />;
    }}</For>
  </svg>;
}
