/**
 * Event-driven notification routing (WP-A19, Z-84 + Z-86): hub
 * `session_changed` cards in, decisions out — OS notifications, needs-you
 * drawer items and the "seen in foreground" bookkeeping. No polling: the
 * hub's events stream is the only source.
 *
 * Rules (ruled hybrid):
 * - **Seed on first look**: a session's first sighting (cold `sessions_list`
 *   or the first event after a restart/reconnect) only records the baseline
 *   — old waits and completions are never replayed as notifications.
 * - **Transitions**: `needs_you` rising → *waiting*; `working → idle` →
 *   *finished*; `working → crashed` → *crashed*. A takeover is none of those.
 * - **Suppression**: the session the conversation shows never notifies
 *   (Z-84), and while the window is in front nothing notifies at all (the
 *   bell and sidebar are visible; unipi's kept rule).
 * - **Max 3** per drain (unipi rule).
 * - **Z-86**: a completion that arrives *while* its session is shown and the
 *   window focused is not unread — but if the window blurs or hides right
 *   after, it is marked unread (persisted host meta), and cleared again when
 *   the user returns to it.
 */
import type { SessionCard } from '../host/client';
import { sessionTitle } from '../session/selectors';
import { groupFor, needsYouId, type NeedsYouItem } from '../needsYou';

export type SessionRef = { hostId: string; sessionId: string };
/** What a card transition is worth attention for. */
export type AttentionKind = 'waiting' | 'finished' | 'crashed';
/** One card change the router decided is worth a look. */
export type Attention = { kind: AttentionKind; hostId: string; hostName: string; card: SessionCard; prev: SessionCard | undefined };
/** What to show as an OS notification (already filtered + capped). */
export type PlannedNotification = Attention & { title: string; body: string };

/** At most this many OS notifications per drain (unipi max-3 rule). */
export const NOTIFY_MAX = 3;

/** The attention a card change carries; `undefined` for none / first sighting. Pure. */
export function attentionOf(prev: SessionCard | undefined, card: SessionCard): AttentionKind | undefined {
  if (!prev) return undefined; // first sighting: seed only (no replay of cold snapshots)
  if (!prev.needs_you && card.needs_you) return 'waiting';
  if (prev.run_state === 'working' && card.run_state === 'idle') return 'finished';
  if (prev.run_state === 'working' && card.run_state === 'crashed') return 'crashed';
  return undefined;
}

/** The OS notification's text (unipi wording; the title is neutral "UniPi" natively). */
export function attentionText(kind: AttentionKind, card: SessionCard, hostName: string): { title: string; body: string } {
  const title = sessionTitle(card);
  if (kind === 'finished') return { title: `${title} finished`, body: `pi on ${hostName} is done.` };
  if (kind === 'crashed') return { title: `${title} stopped`, body: card.error?.trim() || `pi on ${hostName} crashed.` };
  return { title: `${title} needs you`, body: `pi on ${hostName} is waiting for your answer.` };
}

export type PlanOptions = { focused?: SessionRef; windowFocused: boolean; max?: number };

/**
 * Which attentions become OS notifications: the focused session is
 * suppressed, so is everything while the window is in front; at most `max`
 * (default 3) survive. Pure.
 */
export function planNotifications(attentions: readonly Attention[], opts: PlanOptions): PlannedNotification[] {
  if (opts.windowFocused) return [];
  const kept = attentions.filter(
    (a) => !(opts.focused && opts.focused.hostId === a.hostId && opts.focused.sessionId === a.card.session_id),
  );
  return kept.slice(0, Math.max(0, opts.max ?? NOTIFY_MAX)).map((a) => ({ ...a, ...attentionText(a.kind, a.card, a.hostName) }));
}

/** The drawer item an attention becomes (Z-83: the Unicrab drawer's feed). */
export function attentionItem(attention: Attention, since: number): NeedsYouItem {
  const kind = attention.kind === 'waiting' ? 'input_needed' : attention.kind === 'finished' ? 'agent_end' : 'crashed';
  return {
    id: needsYouId(attention.hostId, null, kind, since),
    hostId: attention.hostId,
    hostName: attention.hostName,
    pid: null,
    session: attention.card.session_id,
    title: sessionTitle(attention.card),
    kind,
    group: groupFor(kind),
    what: attention.kind === 'crashed' ? attention.card.error?.trim() || undefined : undefined,
    since,
  };
}

/**
 * Z-86 bookkeeping: completions seen while their session was shown and the
 * window focused. On blur/hide the runner drains these and marks the
 * sessions unread (`session_meta_set`); returning to a still-shown session
 * clears it again (the host clears `unread` on `session_focus`).
 */
export class ForegroundCompletions {
  private readonly seen = new Map<string, SessionRef>();

  private static key(ref: SessionRef): string {
    return `${ref.hostId}/${ref.sessionId}`;
  }

  get size(): number {
    return this.seen.size;
  }

  /** A completion arrived in front of the user (window focused, session shown). */
  record(ref: SessionRef): void {
    this.seen.set(ForegroundCompletions.key(ref), ref);
  }

  /** The shown session changed (or closed): its pending record is moot. */
  forget(ref: SessionRef): void {
    this.seen.delete(ForegroundCompletions.key(ref));
  }

  /** Drains: everything to mark unread now that the user left the window. */
  takeAll(): SessionRef[] {
    const out = [...this.seen.values()];
    this.seen.clear();
    return out;
  }
}

/**
 * The router's per-app state: the last card seen per session (the seed
 * baseline) plus the Z-86 records. `observe` is called once per hub
 * `session_changed` and per cold `sessions_list` card.
 */
export class NotifyRouter {
  private readonly prev = new Map<string, SessionCard>();
  readonly foreground = new ForegroundCompletions();

  private static key(hostId: string, sessionId: string): string {
    return `${hostId}/${sessionId}`;
  }

  /** One card in: the attention it carries (or none), and whether it seeded. */
  observe(hostId: string, hostName: string, card: SessionCard): { attention?: Attention; seeded: boolean } {
    const key = NotifyRouter.key(hostId, card.session_id);
    const prev = this.prev.get(key);
    this.prev.set(key, card);
    const kind = attentionOf(prev, card);
    return kind ? { attention: { kind, hostId, hostName, card, prev }, seeded: false } : { seeded: !prev };
  }

  /** A session vanished (removed/archived): drop its baseline. */
  forget(hostId: string, sessionId: string): void {
    this.prev.delete(NotifyRouter.key(hostId, sessionId));
    this.foreground.forget({ hostId, sessionId });
  }
}
