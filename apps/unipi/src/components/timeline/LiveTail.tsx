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
 */
import { For, Show } from 'solid-js';
import type { OutboxItem } from '../../lib/session/types';
import { outboxLabel } from '../../lib/session/selectors';
import type { Turn } from '../../lib/session/turns';
import { UserBubble } from './UserBubble';
import { TurnRow } from './TurnRow';
import type { TimelineCtx } from './context';

function OutboxBubble(props: { item: OutboxItem; ctx: TimelineCtx }) {
  const status = () => props.item.status;
  return (
    <div class="tl-outbox" data-status={status()} data-testid="tl-outbox">
      <UserBubble
        chat={props.ctx.refs}
        text={props.item.text}
        images={[]}
        pending={status() === 'waiting' || status() === 'lost' ? 'waiting' : 'sending'}
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
  return (
    <div class="tl-tail" data-testid="tl-tail">
      <Show when={props.live}>{(turn) => <TurnRow turn={turn()} ctx={props.ctx} />}</Show>
      <For each={outbox()}>{(item) => <OutboxBubble item={item} ctx={props.ctx} />}</For>
    </div>
  );
}
