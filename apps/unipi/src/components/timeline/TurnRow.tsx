/**
 * One turn render unit (WP-A6, §3.5): the virtual scroll unit — a user
 * query plus everything its run produced, never split by the virtualizer.
 *
 *   preamble     → work nodes only (cards before the first user message)
 *   compaction   → the Z-35 divider
 *   turn         → [cron label] user bubble | command/harness row
 *                  per segment: [steer bubble] fold + answer prose
 *                  tail: turn actions + "Done in 1m 34s · $0.003" + time
 *
 * `/compact` (and other hidden command echoes) show no bubble (Z-35); a
 * non-hidden command turn is a light boundary row (Z-36).
 *
 * WP-A7 adds the turn tail (`TurnActions`), Edit on the user bubble and the
 * per-message row menus (user + prose). The turn is passed its `last` flag so
 * only the final settled turn offers Retry (Z-49).
 */
import { Show } from 'solid-js';
import { Clock } from 'lucide-solid';
import { imagesOf, textOf, type Part } from '../../lib/chat/model';
import type { Segment, Turn } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import { entryIdOfKey, messageText, type MessageTarget } from '../../lib/timeline/actions';
import { SegmentFold, WorkNodes } from './SegmentFold';
import { HarnessRow } from './HarnessRow';
import { UserBubble } from './UserBubble';
import { Prose } from './Prose';
import { ByKey } from './ByKey';
import { Compaction } from './Compaction';
import { TurnActions, requestEdit } from './TurnActions';
import { TurnFooters } from '../../ui/slots/TurnFooters';
import type { TimelineCtx } from './context';

export function TurnRow(props: { turn: Turn; ctx: TimelineCtx; last?: boolean }) {
  const t = () => props.turn;
  const userMenu = (entry: SessionEntry | undefined) => {
    if (!entry) return undefined;
    const target = (): MessageTarget => ({
      text: messageText(entry),
      role: 'user',
      entryId: entry.id,
      userEntryId: entry.id,
      turnId: t().turnId,
    });
    return target;
  };
  const answerMenu = (entryKey: string | undefined, text: string) => {
    const entryId = entryIdOfKey(props.ctx.session.state, entryKey);
    return (): MessageTarget => ({
      text,
      role: 'assistant',
      entryId,
      userEntryId: t().user?.id,
      turnId: t().turnId,
    });
  };
  const onEdit = (entry: SessionEntry | undefined) =>
    entry && !t().live ? (anchor: DOMRect) => void requestEdit(props.ctx, t(), entry, anchor) : undefined;
  return (
    <div class="tl-turn" data-kind={t().kind} data-live={t().live ? '' : undefined} data-testid={`tl-turn-${t().turnId}`}>
      <Show
        when={t().kind !== 'compaction'}
        fallback={<Compaction entry={t().compaction!} sessionId={props.ctx.sessionId} />}
      >
        <Show when={t().kind === 'turn'}>
          <TurnHead turn={t()} ctx={props.ctx} userMenu={userMenu} onEdit={onEdit} />
        </Show>
        <ByKey each={t().segments} key={(s) => s.id}>
          {(seg) => (
            <SegmentView
              segment={seg()}
              ctx={props.ctx}
              preamble={t().kind === 'preamble'}
              turn={t()}
              userMenu={userMenu}
              answerMenu={answerMenu}
              onEdit={onEdit}
            />
          )}
        </ByKey>
        <TurnActions turn={t()} ctx={props.ctx} last={props.last} />
        {/* U10: `turnFooter` renderer entries render inside the turn's row (§6.7). */}
        <TurnFooters ctx={props.ctx} turn={t()} />
      </Show>
    </div>
  );
}

/** The turn's trigger: user bubble, command/harness row, or nothing (hidden echo). */
function TurnHead(props: {
  turn: Turn;
  ctx: TimelineCtx;
  userMenu: (entry: SessionEntry | undefined) => (() => MessageTarget) | undefined;
  onEdit: (entry: SessionEntry | undefined) => ((anchor: DOMRect) => void) | undefined;
}) {
  const t = () => props.turn;
  const m = () => t().user?.message;
  const text = () => textOf(m()?.content as string | Part[] | undefined);
  const images = () => imagesOf(m()?.content as string | Part[] | undefined);
  const hidden = () => !!t().command?.hidden;
  /** A cron run's chat: its first user message is the job's prompt (the scheduler sent it). */
  const scheduled = () =>
    !!props.ctx.cronRun?.() && props.ctx.session.turns().find((x) => x.kind === 'turn')?.turnId === t().turnId;
  return (
    <Show when={!hidden()}>
      <Show when={t().source === 'cron' || scheduled()}>
        <div class="tl-cron" data-testid="tl-cron">
          <Clock size={12} />
          <span>{scheduled() ? 'Scheduled run' : 'Sent by cron'}</span>
        </div>
      </Show>
      <Show
        when={t().source !== 'harness'}
        fallback={<HarnessRow turn={t()} ctx={props.ctx} />}
      >
        <UserBubble
          chat={props.ctx.refs}
          text={text()}
          images={images()}
          commands={props.ctx.commands}
          menu={props.userMenu(t().user)}
          onEdit={props.onEdit(t().user)}
          composerKey={props.ctx.session.key}
          foldKey={{ sessionId: props.ctx.sessionId(), key: `bubble:${t().user?.id ?? t().turnId}` }}
        />
      </Show>
    </Show>
  );
}

function SegmentView(props: {
  segment: Segment;
  ctx: TimelineCtx;
  preamble?: boolean;
  turn: Turn;
  userMenu: (entry: SessionEntry | undefined) => (() => MessageTarget) | undefined;
  answerMenu: (entryKey: string | undefined, text: string) => () => MessageTarget;
  onEdit: (entry: SessionEntry | undefined) => ((anchor: DOMRect) => void) | undefined;
}) {
  const seg = () => props.segment;
  const steerText = () => textOf(seg().steer?.message?.content as string | Part[] | undefined);
  const steerImages = () => imagesOf(seg().steer?.message?.content as string | Part[] | undefined);
  return (
    <div class="tl-segment" data-live={seg().live ? '' : undefined} data-testid={seg().steer ? 'tl-steer' : undefined}>
      <Show when={seg().steer}>
        <UserBubble
          chat={props.ctx.refs}
          text={steerText()}
          images={steerImages()}
          steer
          commands={props.ctx.commands}
          menu={props.userMenu(seg().steer)}
          onEdit={props.onEdit(seg().steer)}
          composerKey={props.ctx.session.key}
          foldKey={{ sessionId: props.ctx.sessionId(), key: `bubble:${seg().steer?.id ?? seg().id}` }}
        />
      </Show>
      <Show when={seg().before?.length}>
        {/* [chat-events] events that opened the segment (recall, nudges) stay visible above the fold */}
        <WorkNodes nodes={seg().before!} ctx={props.ctx} live={false} />
      </Show>
      <Show when={!props.preamble && !!seg().fold}>
        <SegmentFold segment={seg()} ctx={props.ctx} />
      </Show>
      <Show when={props.preamble}>
        {/* preamble: loose work without a user query above renders directly (no "Worked" fold) */}
        <WorkNodes nodes={seg().nodes} ctx={props.ctx} live={seg().live} />
      </Show>
      <Show when={seg().answer}>
        {(answer) => (
          <Prose
            text={answer().text}
            live={answer().live}
            entryKey={answer().key}
            chat={props.ctx.refs}
            width={props.ctx.columnWidth}
            menu={props.answerMenu(answer().key, answer().text)}
            composerKey={props.ctx.session.key}
          />
        )}
      </Show>
      <Show when={seg().after?.length}>
        <WorkNodes nodes={seg().after!} ctx={props.ctx} live={false} />
      </Show>
    </div>
  );
}
