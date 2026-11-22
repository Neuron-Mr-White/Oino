/**
 * One turn render unit (WP-A6, §3.5): the virtual scroll unit — a user
 * query plus everything its run produced, never split by the virtualizer.
 *
 *   preamble     → work nodes only (cards before the first user message)
 *   compaction   → the Z-35 divider
 *   turn         → [cron label] user bubble | command/harness row
 *                  per segment: [steer bubble] fold + answer prose [+ ask_user card]
 *                  tail: turn actions + "Done in 1m 34s · $0.003" + time
 *
 * `/compact` (and other hidden command echoes) show no bubble (Z-35); a
 * non-hidden command turn is a light boundary row (Z-36).
 *
 * WP-A7 adds the turn tail (`TurnActions`), Edit on the user bubble and the
 * per-message row menus (user + prose). [chat-tree] Every user message gets its action row
 * (`UserMessageActions`) and every settled turn its full tail (Retry / Return here on any
 * turn); `last` only keeps the phone's last tail always visible.
 */
import { createEffect, createMemo, For, Show } from 'solid-js';
import { Clock, Loader2 } from 'lucide-solid';
import { imagesOf, textOf, type Part } from '../../lib/chat/model';
import { afterRuns, type AfterRun, type Segment, type Turn } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import { entryIdOfKey, messageText, type MessageTarget } from '../../lib/timeline/actions';
import { needsFullMessage, stripTruncationNote } from '../../lib/session/fullMessages';
import { SegmentFold, WorkNodes } from './SegmentFold';
import { HarnessRow } from './HarnessRow';
import { UserBubble } from './UserBubble';
import { Prose } from './Prose';
import { ByKey } from './ByKey';
import { RenderCutoff } from '../chat/RenderCutoff';
import { Compaction } from './Compaction';
import { FullMessageTail } from './FullMessageTail';
import { AskUserCard } from './cards/AskUserCard';
import { TurnActions, UserMessageActions, requestEdit } from './TurnActions';
import { TurnFooters } from '../../ui/slots/TurnFooters';
import type { TimelineCtx } from './context';

export { afterRuns, type AfterRun } from '../../lib/session/turns';

type Img = ReturnType<typeof imagesOf>[number];
const sameImages = (a: Img[], b: Img[]) =>
  a.length === b.length && a.every((x, i) => x.mime === b[i]!.mime && x.data === b[i]!.data && x.mediaRef === b[i]!.mediaRef && x.bytes === b[i]!.bytes);

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
  // Hotfix 1.0.9: a row showing a message the host cut pulls its full entry (one batched
  // `get_entries{ids}` per burst of rows, see lib/session/fullMessages.ts).
  const cutIds = createMemo(
    () => t().entries.flatMap((e) => (needsFullMessage(e) ? [e.id] : [])),
    [],
    { equals: (a, b) => a.length === b.length && a.every((id, i) => id === b[i]) },
  );
  /** Cut entries no bubble / answer shows a tail for (a thinking or work-prose part, a custom message). */
  const otherCutIds = () => {
    const shown = new Set<string | undefined>([t().user?.id]);
    for (const seg of t().segments) {
      shown.add(seg.steer?.id);
      if (seg.answer) shown.add(entryIdOfKey(props.ctx.session.state, seg.answer.key));
    }
    return cutIds().filter((id) => !shown.has(id));
  };
  createEffect(() => {
    const ids = cutIds();
    if (ids.length) props.ctx.session.fullMessages?.ensure(ids);
  });
  const onEdit = (entry: SessionEntry | undefined) =>
    entry && !t().live && !props.ctx.readOnly ? (anchor: DOMRect) => void requestEdit(props.ctx, t(), entry, anchor) : undefined;
  return (
    <div class="tl-turn" data-kind={t().kind} data-live={t().live ? '' : undefined} data-testid={`tl-turn-${t().turnId}`}>
      <Show when={t().renderCutoff}><RenderCutoff /></Show>
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
        <For each={otherCutIds()}>{(id) => <FullMessageTail ctx={props.ctx} entryId={id} />}</For>
        <Show when={!props.ctx.readOnly}>
          <TurnActions turn={t()} ctx={props.ctx} last={props.last} />
        </Show>
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
  // perf-webview: the live turn object is rebuilt on every stream flush; memo by value so the
  // bubble (segments, fold measure) does not re-render for a user message that did not change.
  const text = createMemo(() => stripTruncationNote(textOf(m()?.content as string | Part[] | undefined)));
  const images = createMemo(() => imagesOf(m()?.content as string | Part[] | undefined), undefined, { equals: sameImages });
  const hidden = () => !!t().command?.hidden;
  /** A cron run's chat: its first user message is the job's prompt (the scheduler sent it). */
  const scheduled = () =>
    !!props.ctx.cronRun?.() && props.ctx.session.turns().find((x) => x.kind === 'turn')?.turnId === t().turnId;
  return (
    <Show when={!hidden()}>
      <Show when={t().source === 'cron' || scheduled()}>
        <div class="tl-cron" data-testid="tl-cron">
          <Clock size={12} />
          <span>{scheduled() ? 'Scheduled run' : 'Sent by an automation'}</span>
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
          actions={t().user && !props.ctx.readOnly ? <UserMessageActions ctx={props.ctx} turn={t()} entry={t().user!} /> : undefined}
          composerKey={props.ctx.session.key}
          foldKey={{ sessionId: props.ctx.sessionId(), key: `bubble:${t().user?.id ?? t().turnId}` }}
        />
        <FullMessageTail ctx={props.ctx} entryId={t().user?.id} />
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
  const steerText = createMemo(() => stripTruncationNote(textOf(seg().steer?.message?.content as string | Part[] | undefined)));
  const steerImages = createMemo(() => imagesOf(seg().steer?.message?.content as string | Part[] | undefined), undefined, { equals: sameImages });
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
          actions={seg().steer && !props.ctx.readOnly ? <UserMessageActions ctx={props.ctx} turn={props.turn} entry={seg().steer!} /> : undefined}
          composerKey={props.ctx.session.key}
          foldKey={{ sessionId: props.ctx.sessionId(), key: `bubble:${seg().steer?.id ?? seg().id}` }}
        />
        <FullMessageTail ctx={props.ctx} entryId={seg().steer?.id} />
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
      <Show when={seg().answer}>
        {(answer) => (
          <>
            {/* D10: a cached excerpt of a long answer says so, in the same words as the host-cut tail, until the live text lands */}
            <Show when={answer().cut}>
              <div class="tl-fullmsg" role="status" data-testid="cached-excerpt">
                <Loader2 size={12} class="tool-more-spin" />
                Loading the rest…
              </div>
            </Show>
            <FullMessageTail ctx={props.ctx} entryId={entryIdOfKey(props.ctx.session.state, answer().key)} />
          </>
        )}
      </Show>
      <Show when={seg().after?.length}>
        <ByKey each={afterRuns(seg().after!)} key={(run) => run.key}>
          {/* G3-2: reads `run()` inside the JSX — a one-time `const item = run()` froze the tool arguments the row was
              mounted with, so a call hydrated afterwards kept showing its clipped copy. */}
          {(run) => (
            <Show
              when={run().kind === 'work' ? (run() as Extract<AfterRun, { kind: 'work' }>) : undefined}
              fallback={<WorkNodes nodes={[(run() as Extract<AfterRun, { kind: 'plain' }>).node]} ctx={props.ctx} live={false} />}
            >
              {(item) => (
                <SegmentFold segment={{ id: `${seg().id}:after:${item().key}`, index: seg().index, work: [], nodes: item().nodes, live: false, fold: { state: 'done', label: 'Worked', detail: item().detail, forcedOpen: false } }} ctx={props.ctx} />
              )}
            </Show>
          )}
        </ByKey>
      </Show>
      <Show when={seg().ask}>
        {/* [ask-user] the question card, where it was asked (the segment's last event, so after the work trailing the answer) — never inside the fold */}
        {(ask) => <AskUserCard tool={ask()} ctx={props.ctx} />}
      </Show>
    </div>
  );
}
