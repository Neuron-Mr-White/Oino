/**
 * One session row of the sidebar / phone Chat list (Z-12, WP-A4): state glyph
 * + word (DESIGN §1.3 colour+icon+word), mode badge, title with a fade mask
 * (no marquee, no ellipsis — full title in the tooltip), relative time (Z-10),
 * `+N −M` changes, the unread dot (wm-green, "done unseen") and the cron
 * clock badge slot.
 *
 * [ux-sessions] layout (owner's zcode-style Task list, 2026-11-03): ONE line — a fixed 16 px
 * gutter that is empty for a normal chat (no chat bubble) and only shows a KIND or STATE icon
 * (cron clock; Working spinner, Needs you, failed / timed out), the title (fade mask), the chips
 * the Properties menu turns on, and the age right-aligned and muted ("63d"; a live state word
 * like "Working" takes its place). Compact view only tightens the row height.
 */
import { createEffect, Match, onCleanup, Show, Switch, type JSX } from 'solid-js';
import { Archive, CircleAlert, CircleDashed, Clock, Cpu, Folder, LoaderCircle, MessageCircleQuestion, Pin, PinOff, TimerOff } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { hostClient } from '../../lib/host/factory';
import { LONG_PRESS_MS } from '../../lib/gestures';
import { menuAnchor } from '../../lib/layout';
import { changesOf, isHomeCwd, sessionTitle } from '../../lib/session/selectors';
import type { SessionMeta } from '../../lib/session/hub';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { openRowMenu } from '../../lib/session/actions';
import { shortAge } from '../../lib/time';
import { cronRunFailed, cronRunLabel } from '../../lib/cronRunLabels';
import { DEFAULT_PROPS, type PropKey } from '../../lib/session/listView';
import { activityTitle, cardActivity, liveActivity, type ActivityFacts } from '../../lib/session/activity';

export type RowVariant = 'sidebar' | 'page';

/** A row's state (DESIGN §1.3, [status] activity states): needs-you beats working (a run, or
 *  work that will wake the session) beats crashed/taken-over beats the quiet background marker. */
export type RowState = 'needs-you' | 'working' | 'crashed' | 'taken-over' | 'background' | 'idle';

function withCardFailure(card: SessionCard, facts: ActivityFacts): RowState {
  if (facts.activity === 'needs-you' || facts.activity === 'working') return facts.activity;
  if (card.run_state === 'crashed') return 'crashed';
  if (card.run_state === 'taken_over') return 'taken-over';
  return facts.activity;
}

/** The row's state from the host card alone. */
export function rowState(card: SessionCard): RowState {
  return withCardFailure(card, cardActivity(card));
}

/** The card reconciled with a live open session stream (`lib/session/activity.ts`): opening an
 *  idle session never flashes "Working", and a parent waiting on subagents / graph nodes stays "Working". */
export function liveRowState(card: SessionCard, open: SessionHandle | undefined): RowState {
  return withCardFailure(card, liveActivity(card, open?.state));
}

/** {@link liveRowState} plus the tooltip of the state (what it waits on / runs). */
export function liveRowFacts(card: SessionCard, open: SessionHandle | undefined): { state: RowState; title?: string } {
  const facts = liveActivity(card, open?.state);
  const state = withCardFailure(card, facts);
  return { state, ...(state === facts.activity && activityTitle(facts) ? { title: activityTitle(facts) } : {}) };
}

export const STATE_WORD: Record<RowState, string> = {
  'needs-you': 'Needs you',
  working: 'Working',
  crashed: 'Crashed',
  'taken-over': 'Taken over',
  // The quiet marker: a glyph and a tooltip, never a word (the age stays).
  background: '',
  idle: '',
};

/** Shared ResizeObserver so `.srow-title` only fades when text genuinely overflows. */
let titleRo: ResizeObserver | undefined;
export function syncTitleOverflow(el: HTMLElement): void {
  const over = el.scrollWidth > el.clientWidth + 1;
  if (over !== el.hasAttribute('data-overflow')) el.toggleAttribute('data-overflow', over);
}
export function observeTitleOverflow(el: HTMLElement): void {
  if (typeof ResizeObserver === 'undefined') return;
  titleRo ??= new ResizeObserver((entries) => {
    for (const entry of entries) syncTitleOverflow(entry.target as HTMLElement);
  });
  titleRo.observe(el);
  onCleanup(() => titleRo?.unobserve(el));
}

/** The working mode badge (Z-12 keeps unipi's): a slot until A16's status pane exposes the live mode. */
export type ModeBadge = { mode: string; label: string; paused?: boolean };

/** Which meta chips a row shows (the Properties menu's "Property visibility"). */
export type RowProps = Record<PropKey, boolean>;

/** The status a row's glyph + chip show: the live state, refined by a cron run's outcome. */
export type RowStatus = RowState | 'failed' | 'timed-out';

const STATUS_WORD: Record<RowStatus, string> = { ...STATE_WORD, failed: 'Failed', 'timed-out': 'Timed out' };

export function SessionRow(props: {
  hostId: string;
  card: SessionCard;
  meta?: SessionMeta;
  active?: boolean;
  variant: RowVariant;
  /** The open session's live mode badge, when the app knows it (LRU sessions). */
  mode?: ModeBadge;
  /** Show the project folder chip even when the Workspace property is off (the Pinned group
   *  spans projects; the phone always does, §3.9). */
  showFolder?: boolean;
  /** Meta chips to show (default: time, status, automation, archived). */
  show?: RowProps;
  /** One line per row: title + icon chips (the Properties menu's Compact view). */
  compact?: boolean;
  /** An archived chat (the Archived filter lists the trash too). */
  archived?: boolean;
  onPin?: (card: SessionCard, pinned: boolean) => void;
}): JSX.Element {
  const open = () => sessions().peek(props.hostId, props.card.session_id);
  const facts = () => liveRowFacts(props.card, open());
  const state = () => facts().state;
  const show = (k: PropKey) => (props.show ?? DEFAULT_PROPS)[k];
  const changes = () => changesOf(props.card);
  const at = () => props.card.last_activity ?? props.card.updated_at;
  /** [ux-sessions] "63d": the Task list's compact age (the exact time is the tooltip). */
  const age = () => shortAge(at());
  /** The exact moment behind the relative time (hover). */
  const stamp = () => {
    const ms = at();
    return ms ? `Updated ${new Date(ms).toLocaleString()}` : undefined;
  };
  const pin = () => (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    props.onPin?.(props.card, !props.meta?.pinned);
  };
  // Z-16 (WP-A5): right-click (desktop, at pointer, D-16) / long-press (phone) opens the row's
  // context menu — one menu app-wide.
  const menuAt = (el: HTMLElement) => openRowMenu(props.hostId, props.card, el.getBoundingClientRect());
  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    if (props.archived) return;
    openRowMenu(props.hostId, props.card, menuAnchor(e, e.currentTarget as HTMLElement));
  };
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  const cancelPress = () => {
    if (pressTimer !== undefined) clearTimeout(pressTimer);
    pressTimer = undefined;
  };
  const onPointerDown = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    if (props.archived || e.pointerType !== 'touch' || e.button !== 0) return;
    const row = e.currentTarget;
    cancelPress();
    pressTimer = setTimeout(() => {
      pressTimer = undefined;
      menuAt(row);
    }, LONG_PRESS_MS);
  };
  const pinTitle = () => (props.meta?.pinned ? `Unpin "${titleText()}"` : `Pin "${titleText()}"`);
  const cron = () => props.card.source === 'cron';
  /** F-11: a cron run's job name and outcome. */
  const cronRun = () => (cron() ? cronRunLabel(props.hostId, props.card.session_id) : undefined);
  /** needs-you / working win; an idle cron run that failed or timed out says so. */
  const status = (): RowStatus => {
    const s = state();
    if (s !== 'idle' && s !== 'background') return s;
    const run = cronRun();
    if (run?.status === 'timeout') return 'timed-out';
    if (cronRunFailed(run)) return 'failed';
    return s;
  };
  const statusWord = () => STATUS_WORD[status()];
  const statusTip = () => {
    if (status() === 'background' || status() === 'working') return facts().title ?? (statusWord() || undefined);
    const word = statusWord();
    if (!word) return undefined;
    if ((status() === 'failed' || status() === 'timed-out') && cronRun()?.reason) return `${word}: ${cronRun()!.reason}`;
    if (status() === 'crashed' && props.card.error) return `${word}: ${props.card.error}`;
    return word;
  };
  const titleText = () => {
    const named = open()?.state.name?.trim() || props.card.name?.trim();
    return !named && cronRun() ? cronRun()!.jobName : sessionTitle(props.card, open()?.state, 'New chat');
  };
  const folder = () => (!isHomeCwd(props.card.cwd) ? props.card.cwd?.split(/[\\/]/).filter(Boolean).pop() : undefined);
  const folderChip = () => {
    const f = folder();
    if (!f || f === titleText()) return undefined;
    return show('workspace') || props.showFolder ? f : undefined;
  };
  const model = () => {
    if (!show('model')) return undefined;
    const live = open()?.state.model;
    const id = (live?.id ? String(live.id) : '') || props.card.model || '';
    return id ? (id.includes('/') ? id.split('/').pop() : id) : undefined;
  };
  const cronTip = () => {
    const run = cronRun();
    return run ? `Automation · ${run.jobName}` : 'Automation (run by the cron scheduler)';
  };
  let titleEl: HTMLSpanElement | undefined;
  createEffect(() => {
    titleText();
    if (titleEl && typeof ResizeObserver !== 'undefined') queueMicrotask(() => titleEl && syncTitleOverflow(titleEl));
  });
  /** The meta chips (comfortable: the second line; compact: icons after the title). */
  const chips = () => (
    <>
      <Show when={show('status') && statusWord()}>
        <span class="srow-word" data-state={status()} title={statusTip()}>
          {statusWord()}
        </span>
      </Show>
      <Show when={show('automation') && cron() && status() !== 'idle' && status() !== 'background'}>
        <span class="srow-chip srow-cron" data-failed={status() === 'failed' || status() === 'timed-out' ? '' : undefined} title={cronTip()} aria-label={cronTip()}>
          <Clock size={12} />
        </span>
      </Show>
      <Show when={folderChip()}>
        {(f) => (
          <span class="srow-chip srow-cwd" title={props.card.cwd}>
            <Folder size={11} />
            <span class="srow-chip-text">{f()}</span>
          </span>
        )}
      </Show>
      <Show when={model()}>
        {(m) => (
          <span class="srow-chip srow-model" title={`Model: ${m()}`}>
            <Cpu size={11} />
            <span class="srow-chip-text">{m()}</span>
          </span>
        )}
      </Show>
      <Show when={props.mode}>
        <span class="srow-mode" data-paused={props.mode!.paused ? '' : undefined} title={`Working mode: ${props.mode!.label}`}>
          {props.mode!.label}
        </span>
      </Show>
      <Show when={changes()}>
        <span class="srow-changes" aria-label={`${changes()!.add} lines added, ${changes()!.del} removed`}>
          <span class="add">+{changes()!.add}</span>
          <span class="del">−{changes()!.del}</span>
        </span>
      </Show>
      <Show when={props.archived && show('archived')}>
        <span class="srow-chip srow-archived" title="Archived — restore it from Archived">
          <Archive size={11} />
        </span>
      </Show>
    </>
  );
  const timeEl = () => (
    <Show when={show('updated') && age() && !(show('status') && statusWord())}>
      <span class="srow-time" title={stamp()}>{age()}</span>
    </Show>
  );
  /**
   * [ux-sessions] The row icon only tells session KINDS apart (owner 2026-11-03): a normal chat has
   * none; a cron run shows its clock; a live state (Working spinner, Needs you, failed…) shows its
   * glyph. The gutter keeps its width, so titles stay aligned either way.
   */
  const glyphKind = (): 'state' | 'cron' | undefined => (status() !== 'idle' && !(status() === 'background' && cron()) ? 'state' : cron() ? 'cron' : undefined);
  return (
    <a
      href={`#/s/${encodeURIComponent(props.hostId)}/${encodeURIComponent(props.card.session_id)}`}
      class="srow"
      data-variant={props.variant}
      data-state={state()}
      data-status={status()}
      data-compact={props.compact ? '' : undefined}
      data-unread={props.meta?.unread ? '' : undefined}
      data-cron={cron() ? '' : undefined}
      data-archived={props.archived ? '' : undefined}
      aria-current={props.active ? 'page' : undefined}
      data-testid={`srow-${props.card.session_id}`}
      title={titleText()}
      onContextMenu={onContextMenu}
      onPointerDown={onPointerDown}
      onPointerUp={cancelPress}
      onPointerCancel={cancelPress}
      onPointerLeave={cancelPress}
    >
      <span class="srow-glyph" data-state={status()} data-kind={glyphKind()} aria-hidden={statusTip() || cron() ? undefined : 'true'} title={statusTip() ?? (cron() ? cronTip() : undefined)} aria-label={statusTip() ?? (cron() ? cronTip() : undefined)} role={statusTip() || cron() ? 'img' : undefined}>
        <Switch>
          <Match when={status() === 'working'}>
            <LoaderCircle size={14} class="srow-spin" />
          </Match>
          <Match when={status() === 'needs-you'}>
            <MessageCircleQuestion size={14} />
          </Match>
          <Match when={status() === 'timed-out'}>
            <TimerOff size={14} />
          </Match>
          <Match when={status() === 'background' && !cron()}>
            <CircleDashed size={13} class="srow-bg" />
          </Match>
          <Match when={status() !== 'idle' && status() !== 'background'}>
            <CircleAlert size={14} />
          </Match>
          <Match when={cron()}>
            <Clock size={13} />
          </Match>
        </Switch>
      </span>
      <span
        ref={(el) => {
          titleEl = el;
          observeTitleOverflow(el);
        }}
        class="srow-title"
      >
        {titleText()}
      </span>
      <Show when={props.meta?.unread}>
        <span class="srow-unread" title="Unread" aria-label="Unread" />
      </Show>
      <span class="srow-chips">{chips()}</span>
      {timeEl()}
      <Show when={props.onPin && !props.archived}>
        <button
          type="button"
          class="srow-pin"
          data-pinned={props.meta?.pinned ? '' : undefined}
          aria-label={props.meta?.pinned ? 'Unpin session' : 'Pin session'}
          title={pinTitle()}
          data-testid={`srow-pin-${props.card.session_id}`}
          onClick={pin()}
        >
          <Show when={props.meta?.pinned} fallback={<Pin size={13} />}>
            <PinOff size={13} />
          </Show>
        </button>
      </Show>
    </a>
  );
}

/** The host connection dot's state (offline hosts show it in their header). */
export function hostConnOf(hostId: string): 'connected' | 'connecting' | 'offline' {
  return hostClient(hostId).conn().state;
}
