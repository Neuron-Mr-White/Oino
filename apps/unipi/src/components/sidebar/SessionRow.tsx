/**
 * One session row of the sidebar / phone Chat list (Z-12, WP-A4): state glyph
 * + word (DESIGN §1.3 colour+icon+word), mode badge, title with a fade mask
 * (no marquee, no ellipsis — full title in the tooltip), relative time (Z-10),
 * `+N −M` changes, the unread dot (wm-green, "done unseen") and the cron
 * clock badge slot. Two lines, like the phone: the title (fade) and its time,
 * then the state word (or the project folder when idle) and the badges —
 * 264 px cannot hold all of Z-12 on one line without hiding the title.
 */
import { Show, type JSX } from 'solid-js';
import { Clock, LoaderCircle, Pin, PinOff } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { hostClient } from '../../lib/host/factory';
import { LONG_PRESS_MS } from '../../lib/gestures';
import { changesOf } from '../../lib/session/selectors';
import type { SessionMeta } from '../../lib/session/hub';
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

export const STATE_WORD: Record<ReturnType<typeof rowState>, string> = {
  'needs-you': 'Needs you',
  working: 'Working',
  crashed: 'Crashed',
  'taken-over': 'Taken over',
  idle: '',
};

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
  const state = () => rowState(props.card);
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
  // Z-16 (WP-A5): right-click (desktop) / long-press (phone) opens the row's
  // context menu — one menu app-wide, anchored to this row.
  const menuAt = (el: HTMLElement) => openRowMenu(props.hostId, props.card, el.getBoundingClientRect());
  const onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    menuAt(e.currentTarget as HTMLElement);
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
  const titleText = () => props.card.name?.trim() || folder() || props.card.session_id;
  const folder = () => props.card.cwd?.split(/[\\/]/).filter(Boolean).pop();
  return (
    <a
      href={`#/s/${encodeURIComponent(props.hostId)}/${encodeURIComponent(props.card.session_id)}`}
      class="srow"
      data-variant={props.variant}
      data-state={state()}
      data-unread={props.meta?.unread ? '' : undefined}
      data-cron={props.card.source === 'cron' ? '' : undefined}
      aria-current={props.active ? 'page' : undefined}
      data-testid={`srow-${props.card.session_id}`}
      title={`${titleText()} · ${STATE_WORD[state()] || 'Idle'}${props.card.cwd ? ` · ${props.card.cwd}` : ''}`}
      onContextMenu={onContextMenu}
      onPointerDown={onPointerDown}
      onPointerUp={cancelPress}
      onPointerCancel={cancelPress}
      onPointerLeave={cancelPress}
    >
      <span class="srow-glyph" data-state={state()} aria-hidden="true">
        <Show when={state() === 'working'} fallback="π">
          <LoaderCircle size={13} class="srow-spin" />
        </Show>
      </span>
      <span class="srow-main">
        <span class="srow-title-line">
          <span class="srow-title">{titleText()}</span>
          <Show when={props.meta?.unread}>
            <span class="srow-unread" title="Unread" aria-label="Unread" />
          </Show>
          <span class="srow-time" title={stamp()}>{time()}</span>
        </span>
        <span class="srow-sub">
          <Show when={STATE_WORD[state()]} fallback={<Show when={props.showFolder && folder()}><span class="srow-cwd">{folder()}</span></Show>}>
            <span class="srow-word" data-state={state()}>{STATE_WORD[state()]}</span>
          </Show>
          <Show when={props.variant === 'page' && STATE_WORD[state()] && folder()}>
            <span class="srow-cwd">{folder()}</span>
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
        </span>
      </span>
    </a>
  );
}

/** The host connection dot's state (offline hosts show it in their header). */
export function hostConnOf(hostId: string): 'connected' | 'connecting' | 'offline' {
  return hostClient(hostId).conn().state;
}
