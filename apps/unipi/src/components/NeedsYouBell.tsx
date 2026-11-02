/**
 * The "needs you" bell + drawer (UNI-161 §3; WP-A19): the animated Unicrab
 * with its unread badge opens a bottom sheet grouped Waiting / Finished /
 * Other. Tapping an item opens that session's route; swipe/✕ dismisses one,
 * header actions mark-all-read / clear everything.
 *
 * `NeedsYouFeed` (WP-A19, Z-83/Z-84/Z-86) is the hub-driven source: every
 * paired host's `session_changed` cards run through the notification router
 * (`lib/notifications/route`) into this store, the OS notifications and the
 * Z-86 foreground-completion bookkeeping. No polling anywhere.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show, untrack } from 'solid-js';
import { useLocation } from '@solidjs/router';
import { Bell, ChevronDown, X } from 'lucide-solid';
import { Unicrab } from './Unicrab';
import { mascotLive } from '../lib/mascotLive';
import './mascot.css';
import { BoardSheet } from './board/BoardSheet';
import { groupNeedsYou, needsYou, type NeedsYouGroup, type NeedsYouItem } from '../lib/needsYou';
import { sessionHref, routeSession } from './shell/Sidebar';
import { useHosts, useHostsOptional } from '../hosts/context';
import { hubs } from '../lib/session/hub';
import { can } from '../lib/platform';
import { desktopNotifyOn, nativeNotify } from '../lib/desktop';
import { syncDesktopNotify } from '../lib/notifications/desktopSwitch';
import { openSessionRoute } from '../lib/notifyInit';
import { attentionItem, NotifyRouter, planNotifications, type Attention, type SessionRef } from '../lib/notifications/route';
import './needsYou.css';

const GROUP_LABEL: Record<NeedsYouGroup, string> = { waiting: 'Waiting', finished: 'Finished', other: 'Other' };
const GROUP_ORDER: NeedsYouGroup[] = ['waiting', 'finished', 'other'];

function relativeTime(ms: number, now = Date.now()): string {
  const diff = Math.max(0, now - ms);
  if (diff < 60_000) return 'just now';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function Row(props: { item: NeedsYouItem; onOpen: () => void; onDismiss: () => void }) {
  return (
    <div class="ny-row" data-read={needsYou().isRead(props.item.id) ? '' : undefined}>
      <button type="button" class="ny-row-main" onClick={props.onOpen}>
        <span class="ny-row-top">
          <span class="ny-row-host">{props.item.hostName}</span>
          <span class="ny-row-time">{relativeTime(props.item.since)}</span>
        </span>
        <span class="ny-row-title">{props.item.title}</span>
        <Show when={props.item.what}>
          <span class="ny-row-what truncate">{props.item.what}</span>
        </Show>
      </button>
      <button type="button" class="ny-row-x" aria-label="Dismiss" onClick={props.onDismiss}>
        <X size={16} />
      </button>
    </div>
  );
}

export function NeedsYouDrawer(props: { open: boolean; onClose: () => void }) {
  const hosts = useHosts();
  const store = needsYou();
  const grouped = createMemo(() => groupNeedsYou(store.list()));
  // Finished is folded away while something is waiting (it is the quieter list); the user can still open it.
  const [finishedToggled, setFinishedToggled] = createSignal<boolean | undefined>();
  const finishedOpen = () => finishedToggled() ?? grouped().waiting.length === 0;

  const open = (item: NeedsYouItem) => {
    store.markRead(item.id);
    openNeedsYouItem(item, hosts.hosts(), () => undefined);
    props.onClose();
  };

  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Needs you"
      headerAction={
        <div class="ny-actions">
          <button type="button" class="ny-action btn btn-ghost btn-sm" onClick={() => store.markAllRead()}>
            Mark all read
          </button>
          <button type="button" class="ny-action btn btn-ghost btn-sm" onClick={() => store.clearAll()}>
            Clear
          </button>
          <button type="button" class="ghost-icon-button board-sheet-close" aria-label="Close" onClick={props.onClose}>
            <X size={20} />
          </button>
        </div>
      }
      testId="needs-you-drawer"
    >
      <Show when={store.list().length > 0} fallback={<p class="ny-empty">Nothing needs you right now.</p>}>
        <For each={GROUP_ORDER}>
          {(group) => (
            <Show when={grouped()[group].length > 0}>
              <section class="ny-group">
                <Show
                  when={group === 'finished' && grouped().waiting.length > 0}
                  fallback={<h3 class="ny-group-head">{GROUP_LABEL[group]}</h3>}
                >
                  <h3 class="ny-group-head">
                    <button type="button" class="ny-group-toggle" aria-expanded={finishedOpen()} onClick={() => setFinishedToggled(!finishedOpen())}>
                      {GROUP_LABEL[group]} · {grouped()[group].length}
                      <ChevronDown size={14} class="ny-group-chev" data-open={finishedOpen() ? '' : undefined} />
                    </button>
                  </h3>
                </Show>
                <Show when={group !== 'finished' || finishedOpen()}>
                  <div class="ny-group-rows">
                    <For each={grouped()[group]}>
                      {(item) => <Row item={item} onOpen={() => open(item)} onDismiss={() => store.dismiss(item.id)} />}
                    </For>
                  </div>
                </Show>
              </section>
            </Show>
          )}
        </For>
      </Show>
    </BoardSheet>
  );
}

/** The "needs you" button: the animated Unicrab (UNI-200) in place of a bell icon, badge count kept. */
export function NeedsYouBell(props: { onOpen: () => void }) {
  const hosts = useHostsOptional();
  const live = mascotLive(() => hosts?.hosts() ?? []);
  const unread = () => needsYou().unreadCount();
  return (
    <button type="button" class="ghost-icon-button ny-bell" aria-label="Needs you" data-state={live.state()} onClick={() => (live.poke(), props.onOpen())}>
      <Unicrab state={live.state()} size={30} />
      <Show when={unread() > 0}>
        <span class="ny-badge">{unread() > 9 ? '9+' : unread()}</span>
      </Show>
    </button>
  );
}

/**
 * P1-06: the desktop shell's bell — a compact icon button with the unread
 * badge in the sidebar head (and beside "Show sidebar" when it is collapsed),
 * so pending approvals / questions on every PC are one click away on every
 * screen. The phone keeps the floating Unicrab.
 */
/** The desktop shell bell ([ui-shell] #5): a red dot while anything is unread (clears once read),
 *  the count in its accessible label. */
export function ShellNeedsYouBell(props: { onOpen: () => void }) {
  const unread = () => needsYou().unreadCount();
  const waiting = () => needsYou().list().some((i) => i.group === 'waiting' && !needsYou().isRead(i.id));
  const label = () => (unread() > 0 ? `Notifications, ${unread()} unread` : 'Notifications');
  return (
    <button
      type="button"
      class="shell-icon-btn ny-shell-bell"
      aria-label={label()}
      title={label()}
      data-waiting={waiting() ? '' : undefined}
      data-unread={unread() > 0 ? '' : undefined}
      data-testid="shell-needs-you"
      onClick={() => props.onOpen()}
    >
      <Bell size={16} />
      <Show when={unread() > 0}>
        <span class="ny-shell-dot" aria-hidden="true" data-testid="shell-needs-you-dot" />
      </Show>
    </button>
  );
}

/** Opens the session an item points at (the `/s/:host/:session` route), or the fallback when its host is gone. */
export function openNeedsYouItem(item: NeedsYouItem, hosts: readonly { id: string }[], fallback: () => void): void {
  const host = hosts.find((h) => h.id === item.hostId);
  if (!item.session || !host) return fallback();
  openSessionRoute(item.hostId, item.session);
}

/**
 * The hub-driven needs-you + notification source (WP-A19): mounted once by
 * the shell. Every paired host's hub cards run through `NotifyRouter`:
 * - transitions feed the drawer (`needsYou`) and, on the desktop, become OS
 *   notifications (Z-84: event-driven, seeded, max 3, focused session and
 *   focused window suppressed);
 * - a completion seen in front of the user is remembered and marked unread
 *   when the window blurs right after (Z-86); returning to it clears the
 *   dot again (`session_focus`).
 */
export function NeedsYouFeed() {
  const hosts = useHosts();
  const location = useLocation();
  const store = needsYou();
  const router = new NotifyRouter();
  // One desktop switch: follow the notify extension's "Native desktop".
  if (can('nativeNotifications')) void syncDesktopNotify();
  /** The item ids this feed added, per session (so an ended wait leaves the drawer). */
  const added = new Map<string, { waiting?: string; finished?: string }>();

  const shown = createMemo(() => routeSession(location.pathname));
  const windowFocused = () => (typeof document === 'undefined' ? true : document.hasFocus() && !document.hidden);

  const key = (hostId: string, sessionId: string) => `${hostId}/${sessionId}`;
  const sinceOf = (card: { last_activity?: number; updated_at?: number }) => card.last_activity ?? card.updated_at ?? Date.now();

  createEffect(() => {
    const attentions: Attention[] = [];
    const focus = shown();
    for (const host of hosts.hosts()) {
      const hub = hubs().ensure(host.id);
      const cards = Object.values(hub.state.cards);
      for (const card of cards) {
        const { attention } = router.observe(host.id, host.name, card);
        const k = key(host.id, card.session_id);
        // The wait ended / a new run started: its drawer item goes, transition or not.
        const have = added.get(k);
        if (have?.waiting && !card.needs_you) {
          store.remove(have.waiting);
          have.waiting = undefined;
        }
        if (have?.finished && card.run_state === 'working') {
          store.remove(have.finished);
          have.finished = undefined;
        }
        if (!attention) continue;
        const item = attentionItem(attention, sinceOf(card));
        store.upsert(item);
        const slot = attention.kind === 'waiting' ? 'waiting' : 'finished';
        added.set(k, { ...(have ?? {}), [slot]: item.id });
        attentions.push(attention);
        // Z-86: a completion watched in this window isn't unread — until the user leaves.
        if (attention.kind === 'finished' && windowFocused() && focus && focus.hostId === host.id && focus.sessionId === card.session_id) {
          router.foreground.record({ hostId: host.id, sessionId: card.session_id });
        }
      }
      // Sessions that vanished (archived/deleted) leave the drawer.
      for (const [k, ids] of [...added]) {
        if (!k.startsWith(`${host.id}/`)) continue;
        const sessionId = k.slice(host.id.length + 1);
        if (!hub.state.cards[sessionId]) {
          if (ids.waiting) store.remove(ids.waiting);
          if (ids.finished) store.remove(ids.finished);
          added.delete(k);
          router.forget(host.id, sessionId);
        }
      }
    }

    const plan = planNotifications(attentions, { focused: focus, windowFocused: windowFocused() });
    if (can('nativeNotifications') && desktopNotifyOn()) {
      for (const n of plan) void nativeNotify(n.title, n.body);
    }
  });

  // Leaving the shown session moots its pending foreground completion (the
  // user moved on inside the app; only a blur right after marks it, Z-86).
  // Opening a chat (from anywhere) settles its finished notes: they leave the drawer.
  createEffect(() => {
    const now = shown();
    if (now) untrack(() => store.settleSession(now.hostId, now.sessionId));
  });

  createEffect((prev?: SessionRef) => {
    const now = shown();
    if (prev) router.foreground.forget(prev);
    return now;
  });

  const onLeave = (fromBlur: boolean) => () => {
    if (!fromBlur && !document.hidden) return; // visibilitychange: only when hidden
    for (const ref of router.foreground.takeAll()) void hubs().ensure(ref.hostId).setMeta(ref.sessionId, { unread: true });
  };
  const onReturn = () => {
    // Back in front: a still-shown session clears the dot the blur marked.
    const s = shown();
    if (s) hubs().ensure(s.hostId).focus(s.sessionId);
  };
  const onBlur = onLeave(true);
  const onHide = onLeave(false);
  window.addEventListener('blur', onBlur);
  document.addEventListener('visibilitychange', onHide);
  window.addEventListener('focus', onReturn);
  document.addEventListener('visibilitychange', onReturn);
  onCleanup(() => {
    window.removeEventListener('blur', onBlur);
    window.removeEventListener('focus', onReturn);
    document.removeEventListener('visibilitychange', onHide);
    document.removeEventListener('visibilitychange', onReturn);
  });
  return null;
}
