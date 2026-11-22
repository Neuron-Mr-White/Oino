/**
 * Live tail (WP-A6, Z-22): the running turn renders expanded *outside* the
 * virtual list, below it in the same scroll container; on `agent_end` it
 * migrates into the list with its measured height pre-seeded (no jump —
 * `Timeline` seeds `heightCache` before the swap). The "Working for" clock
 * in the live fold is the one live indicator (the old separate RunningRow
 * is gone).
 *
 * Below it: outbox bubbles — prompts sent but not yet confirmed by the host
 * ("Sending…"), queued while the stream is down ("Waiting for connection…
 * ✕"), or lost ("Not sent · Retry" — Z-50).
 *
 * Messages the host holds in its steering / follow-up queues are NOT drawn
 * here: while they wait they live in the Queue dock only, and once delivered
 * they arrive as ordinary user entries in the turn they joined (queue-once).
 */
import { createMemo, For, Show } from 'solid-js';
import type { OutboxItem } from '../../lib/session/types';
import { outboxLabel } from '../../lib/session/selectors';
import { echoedRefs } from '../../lib/session/outbox';
import type { Turn } from '../../lib/session/turns';
import { UserBubble } from './UserBubble';
import { TurnRow } from './TurnRow';
import type { TimelineCtx } from './context';

function OutboxBubble(props: { item: OutboxItem; ctx: TimelineCtx }) {
  const status = () => props.item.status;
  const queued = () => !!props.ctx.session.state.approvals?.length && !['waiting', 'lost'].includes(status());
  return (
    <div class="tl-outbox" data-status={status()} data-testid="tl-outbox">
      <UserBubble
        chat={props.ctx.refs}
        text={props.item.text}
        images={[]}
        pending={queued() ? 'queued' : status() === 'waiting' ? 'waiting' : status() === 'lost' ? 'lost' : status() === 'acked' ? undefined : 'sending'}
        error={status() === 'lost' ? props.item.error : undefined}
        onDismiss={status() === 'waiting' ? () => props.ctx.session.dismiss(props.item.ref) : undefined}
        commands={props.ctx.commands}
      />
      <Show when={status() === 'lost'}>
        <button type="button" class="tl-outbox-retry" onClick={() => props.ctx.session.resend(props.item.ref)}>
          {outboxLabel(status())} · Retry
        </button>
      </Show>
    </div>
  );
}

export function LiveTail(props: { live: Turn | undefined; ctx: TimelineCtx }) {
  const outbox = () => props.ctx.session.state.outbox;
  /** What the host already holds queued: shown by the Queue dock, so its outbox twin stays hidden. */
  const queued = createMemo(() => {
    const q = props.ctx.session.state.queue;
    return [
      ...q.steering.map((text, i) => ({ text, id: q.steeringIds?.[i] })),
      ...q.followUp.map((text, i) => ({ text, id: q.followUpIds?.[i] })),
    ];
  });
  const inQueue = (item: OutboxItem) => queued().some((q) => q.id ? q.id === item.messageId : item.mode !== 'prompt' && q.text === item.text);
  /** D3 + G2-4: the typed `/swarm <text>` is already on screen as its command echo's own bubble (paired by order/time, one echo per item — not by text alone). */
  const echoed = createMemo(() => echoedRefs(outbox(), props.ctx.session.turns()));
  return (
    <div class="tl-tail" data-testid="tl-tail">
      <Show when={props.live}>{(turn) => <TurnRow turn={turn()} ctx={props.ctx} />}</Show>
      <For each={outbox()}>{(item) => <Show when={!echoed().has(item.ref) && !inQueue(item)}><OutboxBubble item={item} ctx={props.ctx} /></Show>}</For>
    </div>
  );
}
