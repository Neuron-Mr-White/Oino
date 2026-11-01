/**
 * One session row of the sidebar / phone Chat list (Z-12, WP-A4): state glyph
 * + word (DESIGN §1.3 colour+icon+word), mode badge, title with a fade mask
 * (no marquee, no ellipsis — full title in the tooltip), relative time (Z-10),
 * `+N −M` changes, the unread dot (wm-green, "done unseen") and the cron
 * clock badge slot. Two lines, like the phone: the title (fade) and its time,
 * then the state word (or the project folder when idle) and the badges —
 * 264 px cannot hold all of Z-12 on one line without hiding the title.
 */
import { createEffect, onCleanup, Show, type JSX } from 'solid-js';
import { CircleAlert, Clock, LoaderCircle, MessageCircleQuestion, MessageSquare, Pin, PinOff } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { hostClient } from '../../lib/host/factory';
import { LONG_PRESS_MS } from '../../lib/gestures';
import { menuAnchor } from '../../lib/layout';
import { changesOf, isHomeCwd, sessionTitle } from '../../lib/session/selectors';
import type { SessionMeta } from '../../lib/session/hub';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { openRowMenu } from '../../lib/session/actions';
import { relativeTimeMs } from '../../lib/time';

export type RowVariant = 'sidebar' | 'page';

/** The row's state (DESIGN §1.3): needs-you beats working beats crashed/taken-over. */
export function rowState(card: SessionCard): 'needs-you' | 'working' | 'crashed' | 'taken-over' | 'idle' {
  if (card.needs_you) return 'needs-you';
  if (card.run_state === 'working') return 'working';
  if (card.run_state === 'crashed') return 'crashed';
  if (card.run_state === 'taken_over') return 'taken-over';
  return 'idle';
}

/** Reconciles the card's polled `run_state` with a live open session stream so opening an idle session never flashes "Working". */
export function liveRowState(card: SessionCard, open: SessionHandle | undefined): ReturnType<typeof rowState> {
  if (card.needs_you) return 'needs-you';
  if (open?.state.conn === 'live') {
    if (open.state.run.running) return 'working';
    if (card.run_state === 'working') return 'idle';
  }
  return rowState(card);
}

export const STATE_WORD: Record<ReturnType<typeof rowState>, string> = {
  'needs-you': 'Needs you',
  working: 'Working',
  crashed: 'Crashed',
  'taken-over': 'Taken over',
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

export function SessionRow(props: {
  hostId: string;
  card: SessionCard;
  meta?: SessionMeta;
  active?: boolean;
  variant: RowVariant;
  /** The open session's live mode badge, when the app knows it (LRU sessions). */
  mode?: ModeBadge;
  /** Show the project folder under the title when there is no state word (the Pinned group
   *  spans projects; the phone always does, §3.9). */
  showFolder?: boolean;
  onPin?: (card: SessionCard, pinned: boolean) => void;
}): JSX.Element {
  const open = () => sessions().peek(props.hostId, props.card.session_id);
  const state = () => liveRowState(props.card, open());
  const changes = () => changesOf(props.card);
  const at = () => props.card.last_activity ?? props.card.updated_at;
  const time = () => relativeTimeMs(at());
  /** The exact moment behind the relative time (hover). */
  const stamp = () => {
    const ms = at();
    return ms ? new Date(ms).toLocaleString() : undefined;
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
    openRowMenu(props.hostId, props.card, menuAnchor(e, e.currentTarget as HTMLElement));
  };
  let pressTimer: ReturnType<typeof setTimeout> | undefined;
  const cancelPress = () => {
    if (pressTimer !== undefined) clearTimeout(pressTimer);
    pressTimer = undefined;
  };
  const onPointerDown = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    if (e.pointerType !== 'touch' || e.button !== 0) return;
    const row = e.currentTarget;
    cancelPress();
    pressTimer = setTimeout(() => {
      pressTimer = undefined;
      menuAt(row);
    }, LONG_PRESS_MS);
  };
  const pinTitle = () => (props.meta?.pinned ? `Unpin "${titleText()}"` : `Pin "${titleText()}"`);
  const titleText = () => sessionTitle(props.card, open()?.state, 'New chat');
  const folder = () => (!isHomeCwd(props.card.cwd) ? props.card.cwd?.split(/[\\/]/).filter(Boolean).pop() : undefined);
  const stateWord = () => STATE_WORD[state()];
  const subFolder = () => {
    const f = folder();
    if (!f || f === titleText()) return undefined;
    return (!stateWord() && props.showFolder) || (props.variant === 'page' && !!stateWord()) ? f : undefined;
  };
  const hasSub = () => Boolean(stateWord() || subFolder() || props.mode || changes() || props.card.source === 'cron');
  let titleEl: HTMLSpanElement | undefined;
  createEffect(() => {
    titleText();
    if (titleEl && typeof ResizeObserver !== 'undefined') queueMicrotask(() => titleEl && syncTitleOverflow(titleEl));
  });
  return (
    <a
      href={`#/s/${encodeURIComponent(props.hostId)}/${encodeURIComponent(props.card.session_id)}`}
      class="srow"
      data-variant={props.variant}
      data-state={state()}
      data-sub={hasSub() ? '' : undefined}
      data-unread={props.meta?.unread ? '' : undefined}
      data-cron={props.card.source === 'cron' ? '' : undefined}
      aria-current={props.active ? 'page' : undefined}
      data-testid={`srow-${props.card.session_id}`}
      title={titleText()}
      onContextMenu={onContextMenu}
      onPointerDown={onPointerDown}
      onPointerUp={cancelPress}
      onPointerCancel={cancelPress}
      onPointerLeave={cancelPress}
    >
      <span class="srow-glyph" data-state={state()} aria-hidden="true">
        <Show
          when={state() === 'working'}
          fallback={
            <Show
              when={state() === 'needs-you'}
              fallback={
                <Show when={state() === 'crashed' || state() === 'taken-over'} fallback={<MessageSquare size={14} />}>
                  <CircleAlert size={14} />
                </Show>
              }
            >
              <MessageCircleQuestion size={14} />
            </Show>
          }
        >
          <LoaderCircle size={14} class="srow-spin" />
        </Show>
      </span>
      <span class="srow-main">
        <span class="srow-title-line">
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
          <span class="srow-time" title={stamp()}>{time()}</span>
          <Show when={props.onPin}>
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
        </span>
        <Show when={hasSub()}>
          <span class="srow-sub">
            <Show when={stateWord()}>
              <span class="srow-word" data-state={state()}>{stateWord()}</span>
            </Show>
            <Show when={subFolder()}>
              <span class="srow-cwd">{subFolder()}</span>
            </Show>
            <Show when={props.mode}>
              {(m) => (
                <span class="srow-mode" data-paused={m().paused ? '' : undefined} title={`Working mode: ${m().label}`}>
                  {m().label}
                </span>
              )}
            </Show>
            <span class="srow-tail">
              <Show when={changes()}>
                {(c) => (
                  <span class="srow-changes" aria-label={`${c().add} lines added, ${c().del} removed`}>
                    <span class="add">+{c().add}</span>
                    <span class="del">−{c().del}</span>
                  </span>
                )}
              </Show>
              <Show when={props.card.source === 'cron'}>
                <span class="srow-cron" title="Run by the cron scheduler">
                  <Clock size={12} />
                  <span class="srow-cron-label">cron</span>
                </span>
              </Show>
            </span>
          </span>
        </Show>
      </span>
    </a>
  );
}

/** The host connection dot's state (offline hosts show it in their header). */
export function hostConnOf(hostId: string): 'connected' | 'connecting' | 'offline' {
  return hostClient(hostId).conn().state;
}
