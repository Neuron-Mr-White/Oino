import { createMemo, For, Show, type Accessor } from 'solid-js';
import { modelDisplayName } from '../../../lib/model/displayName';
import { detailLine, elapsed, nodeTitle, ORB, sizeKind, STATUS_LABEL, stateText, type Density, type FleetTree, type PlacedNode } from '../fleet';
import { ModeChip, Ring } from './parts';

/** Everything a node needs from the canvas, as accessors, so one node never re-renders for another. */
export type NodeCtx = {
  tree: Accessor<FleetTree>;
  now: Accessor<number>;
  lod: Accessor<0 | 1 | 2>;
  density: Accessor<Density>;
  dimmed: Accessor<ReadonlySet<string>>;
  /** Search matches; null while there is no query. */
  match: Accessor<ReadonlySet<string> | null>;
  current: Accessor<string | undefined>;
  animated: Accessor<ReadonlySet<string>>;
  selected: Accessor<string | undefined>;
  tabbable: Accessor<string | undefined>;
  select: (id: string) => void;
  open: (id: string) => void;
  hover: (id: string | undefined) => void;
  menu: (id: string, anchor: DOMRect) => void;
  key: (e: KeyboardEvent, id: string) => void;
  toggle: (owner: string) => void;
  click: (e: MouseEvent) => boolean;
};

const LONG_PRESS = 500;
const ICON = { superagent: 32, lead: 28, sub: 22, chat: 22 } as const;
const TICKETS = 3;

function tip(node: ReturnType<FleetTree['nodes']['get']> & {}, now: number) {
  const model = node.model ? modelDisplayName({ id: node.model }) : '';
  return [nodeTitle(node), stateText(node, now), model, node.step, node.question?.excerpt, node.health?.error].filter(Boolean).join(' · ');
}

export function NodeView(props: { id: string; at: Accessor<PlacedNode | undefined>; ctx: NodeCtx }) {
  const c = props.ctx;
  const node = () => c.tree().nodes.get(props.id);
  const dim = () => c.dimmed().has(props.id);
  const missed = () => { const m = c.match(); return !!m && !m.has(props.id); };
  const mode = () => (c.lod() === 2 ? 'dot' : c.lod() === 1 ? 'compact' : c.density());
  const style = () => { const p = props.at(); return p ? { left: `${p.x}px`, top: `${p.y}px`, width: `${p.w}px`, height: `${p.h}px` } : undefined; };

  let press: ReturnType<typeof setTimeout> | undefined;
  let pressed = false;
  const down = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    if (e.pointerType !== 'touch') return;
    const el = e.currentTarget; pressed = false;
    clearTimeout(press);
    press = setTimeout(() => { pressed = true; c.menu(props.id, el.getBoundingClientRect()); }, LONG_PRESS);
  };
  const cancel = () => clearTimeout(press);

  return <Show when={props.at()?.pseudo ? undefined : node()} fallback={<Stack id={props.id} at={props.at} ctx={c} />}>{(n) => {
    const kind = () => sizeKind(n());
    return <button type="button" class="cn" data-id={props.id} data-testid={`fleet-node-${props.id}`} data-role={n().role} data-status={n().status} data-mode={mode()}
      data-anim={c.animated().has(props.id) ? '' : undefined} data-dim={dim() ? '' : undefined} data-miss={missed() ? '' : undefined}
      data-selected={c.selected() === props.id ? '' : undefined} data-current={c.current() === props.id ? '' : undefined}
      tabIndex={c.tabbable() === props.id ? 0 : -1} data-tip={mode() === 'detailed' ? undefined : tip(n(), c.now())}
      aria-label={`${nodeTitle(n())}, ${STATUS_LABEL[n().status]}${n().model ? `, ${modelDisplayName({ id: n().model! })}` : ''}`} aria-pressed={c.selected() === props.id}
      style={style()}
      onClick={(e) => { if (pressed) { pressed = false; return; } if (c.click(e)) c.select(props.id); }} onDblClick={() => c.open(props.id)}
      onPointerEnter={() => c.hover(props.id)} onPointerLeave={() => { c.hover(undefined); cancel(); }} onFocus={() => c.hover(props.id)} onBlur={() => c.hover(undefined)}
      onPointerDown={down} onPointerUp={cancel} onPointerCancel={cancel} onPointerMove={(e) => { if (e.pointerType === 'touch' && (Math.abs(e.movementX) > 2 || Math.abs(e.movementY) > 2)) cancel(); }}
      onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); c.menu(props.id, new DOMRect(e.clientX, e.clientY, 0, 0)); }}
      onKeyDown={(e) => { if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) { e.preventDefault(); c.menu(props.id, e.currentTarget.getBoundingClientRect()); } else c.key(e, props.id); }}>
      <Show when={mode() === 'dot'}><span class="cn-dot" aria-hidden="true" /></Show>
      <Show when={mode() === 'compact'}>
        <Ring node={n()} size={ORB[kind()]} icon={ICON[kind()]} anim={c.animated().has(props.id)} />
        <span class="cn-label"><strong class="cn-title">{nodeTitle(n())}</strong><span class="cn-state">{stateText(n(), c.now())}</span></span>
      </Show>
      <Show when={mode() === 'detailed'}>
        <Show when={n().status === 'working'}><span class="cn-sweep" aria-hidden="true" /></Show>
        <span class="cn-row1"><Ring node={n()} size={36} icon={16} anim={c.animated().has(props.id)} /><strong class="cn-title">{nodeTitle(n())}</strong><span class="cn-pill">{STATUS_LABEL[n().status]}</span></span>
        <span class="cn-row2"><span class="cn-model">{n().model ? modelDisplayName({ id: n().model! }) : 'Model unknown'}</span><Show when={n().role === 'lead'}><ModeChip mode={n().mode} /></Show></span>
        <Show when={n().role === 'lead' && n().ticketIds.length}><span class="cn-tickets"><For each={n().ticketIds.slice(0, TICKETS)}>{(t) => <code>{t}</code>}</For><Show when={n().ticketIds.length > TICKETS}><code>+{n().ticketIds.length - TICKETS}</code></Show></span></Show>
        <span class="cn-row4"><span class="cn-detail">{detailLine(n())}</span><time class="cn-time">{elapsed(n(), c.now()) ?? ''}</time></span>
      </Show>
    </button>;
  }}</Show>;
}

/** The "+N done" and "+N earlier chats" stacks: one control that folds or unfolds a set of nodes. */
function Stack(props: { id: string; at: Accessor<PlacedNode | undefined>; ctx: NodeCtx }) {
  const p = () => props.at();
  const label = createMemo(() => { const x = p(); if (!x) return ''; const what = x.pseudo === 'earlier' ? 'earlier chats' : 'done'; return x.open ? `Hide ${what}` : `+${x.count} ${what}`; });
  return <button type="button" class="cn-stack" data-id={props.id} data-testid={`fleet-stack-${props.id}`} data-open={p()?.open ? '' : undefined} aria-expanded={!!p()?.open}
    style={p() ? { left: `${p()!.x}px`, top: `${p()!.y}px`, width: `${p()!.w}px`, height: `${p()!.h}px` } : undefined} onClick={(e) => { if (props.ctx.click(e)) props.ctx.toggle(p()?.owner ?? 'earlier'); }}>
    <span class="cn-stack-pile" aria-hidden="true"><i /><i /><i /></span>{label()}
  </button>;
}
