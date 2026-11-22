import { Show, type Accessor } from 'solid-js';
import { ChevronDown, Hand, LayoutDashboard, Moon, TriangleAlert } from 'lucide-solid';
import type { PlacedGroup } from '../fleet';

/** One project (or folder, or "Other chats"): the frame, the header with its counts, the Chats caption. */
export function GroupView(props: { g: Accessor<PlacedGroup | undefined>; lod: Accessor<0 | 1 | 2>; toggle: (key: string) => void; board: (key: string) => void }) {
  const g = () => props.g();
  return <Show when={g()}>{(group) => <div class="cg" data-testid={`fleet-group-${group().key}`} data-collapsed={group().collapsed ? '' : undefined} data-board={group().hasBoard ? '' : undefined} style={{ left: `${group().x}px`, top: `${group().y}px`, width: `${group().w}px`, height: `${group().h}px` }}>
    <header class="cg-head">
      <button type="button" class="cg-toggle" aria-expanded={!group().collapsed} aria-label={`${group().collapsed ? 'Expand' : 'Collapse'} ${group().name}`} onClick={() => props.toggle(group().key)}><ChevronDown size={16} aria-hidden="true" /></button>
      <h3 class="cg-name" title={group().name}>{group().name}</h3>
      <Show when={!group().hasBoard && group().key !== 'other'}><span class="cg-tag">No board</span></Show>
      <Show when={props.lod() < 2}>
        <span class="cg-counts">
          <Show when={group().counts.working}><span class="cg-c" data-status="working" aria-label={`${group().counts.working} working`}><i class="cg-dot" />{group().counts.working}</span></Show>
          <Show when={group().counts.waiting}><span class="cg-c" data-status="waiting" aria-label={`${group().counts.waiting} need you`}><Hand size={12} aria-hidden="true" />{group().counts.waiting}</span></Show>
          <Show when={group().counts.stalled}><span class="cg-c" data-status="stalled" aria-label={`${group().counts.stalled} stalled`}><TriangleAlert size={12} aria-hidden="true" />{group().counts.stalled}</span></Show>
          <Show when={group().counts.idle}><span class="cg-c" data-status="idle" aria-label={`${group().counts.idle} idle`}><Moon size={12} aria-hidden="true" />{group().counts.idle}</span></Show>
          <Show when={group().counts.ended}><span class="cg-c" data-status="ended">{group().counts.ended} done</span></Show>
        </span>
      </Show>
      <Show when={group().hasBoard}><button type="button" class="cg-board" aria-label={`Open ${group().name} board`} data-tip="Open board" onClick={() => props.board(group().key)}><LayoutDashboard size={14} aria-hidden="true" /><span>Open board</span></button></Show>
    </header>
    <Show when={group().chatsLabel && props.lod() < 2}><span class="cg-caption" style={{ left: '20px', top: `${group().chatsLabel!.y - group().y}px` }}>Chats</span></Show>
  </div>}</Show>;
}
