import { Show, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { Bot, BotMessageSquare, Check, Hand, MessageSquare, Moon, Sparkles, TriangleAlert, Workflow, Network } from 'lucide-solid';
import type { FleetNode } from '../api';
import type { FleetStatus } from '../fleet';

/** Role -> icon (OIN-77 §2). Role colour lives in CSS (`.cn[data-role]` sets `--role`). */
export const ROLE_ICON = { superagent: Bot, lead: BotMessageSquare, worker: Workflow, subagent: Sparkles, chat: MessageSquare } as const;
export const STATE_ICON = { waiting: Hand, stalled: TriangleAlert, idle: Moon, ended: Check } as const;

export function RoleIcon(props: { role: FleetNode['role']; size: number }) {
  // Reactive: the peek reuses one ring for whichever node is picked.
  return <Dynamic component={ROLE_ICON[props.role]} size={props.size} aria-hidden="true" />;
}

/** The state badge in the ring's top-right corner (working has none: it moves instead). */
export function StateBadge(props: { status: FleetStatus }) {
  return <Show when={props.status !== 'working'}>
    <span class="cn-badge" data-status={props.status} aria-hidden="true"><Dynamic component={STATE_ICON[props.status as keyof typeof STATE_ICON]} size={10} strokeWidth={2.5} /></span>
  </Show>;
}

/** The role tile inside its state ring. `size` is the ring's outer diameter, `icon` the glyph. */
export function Ring(props: { node: FleetNode; size: number; icon: number; anim: boolean }): JSX.Element {
  return <span class="cn-ring" style={{ '--s': `${props.size}px` }} data-status={props.node.status} data-role={props.node.role} data-anim={props.anim ? '' : undefined}>
    <span class="cn-tile"><span class="cn-icon"><RoleIcon role={props.node.role} size={props.icon} /></span></span>
    <StateBadge status={props.node.status} />
  </span>;
}

/** Mode chips are neutral with an icon (frees cyan and orange for roles). Plain shows nothing. */
export function ModeChip(props: { mode: FleetNode['mode'] }) {
  return <Show when={props.mode !== 'plain'}><span class="cn-mode" data-mode={props.mode}>{props.mode === 'graph' ? <Workflow size={10} aria-hidden="true" /> : <Network size={10} aria-hidden="true" />}{props.mode === 'graph' ? 'Graph' : 'Swarm'}</span></Show>;
}
