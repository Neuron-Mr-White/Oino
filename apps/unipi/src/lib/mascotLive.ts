/**
 * The mascot's live state (UNI-200): one reactive source for the floating
 * crab, the desktop header crab, the desktop mascot window and the Android
 * overlay — derived from the "needs you" store + every host's hub cards
 * (the same data the bell badge and tray already use; no new poll).
 */
import { createMemo, createRoot, createSignal, on, createEffect, type Accessor } from 'solid-js';
import { needsYou } from './needsYou';
import { hubs } from './session/hub';
import { sessions } from './session/store';
import { isInternalSession } from './session/internal';
import { liveActivity } from './session/activity';
import type { HostInfo } from './schemas';
import { mascotState, type MascotInputs, type MascotState } from './unicrab';
import { clockNow } from './clock';

export type MascotLive = { state: Accessor<MascotState>; count: Accessor<number>; inputs: Accessor<MascotInputs>; poke: () => void };

/**
 * Sessions working across `hosts` ([status]: the tray count and the crab) — the hub cards each
 * PC's sidebar shows, reconciled with open live streams (`lib/session/activity.ts`). A parent
 * waiting on subagents / graph nodes / a waking bg task counts; quiet background work does not.
 */
export function runningAgents(hosts: readonly HostInfo[]): number {
  let running = 0;
  for (const host of hosts) {
    const cards = hubs().peek(host.id)?.state.cards;
    if (!cards) continue;
    for (const card of Object.values(cards)) {
      if (isInternalSession(card.session_id)) continue;
      if (liveActivity(card, sessions().peek(host.id, card.session_id)?.state).activity === 'working') running += 1;
    }
  }
  return running;
}

/** Hosts whose session list failed and that never answered. */
export function offlineHosts(hosts: readonly HostInfo[]): number {
  return hosts.filter((h) => {
    const hub = hubs().peek(h.id)?.state;
    return !!hub && hub.error !== undefined && !hub.loaded;
  }).length;
}

function create(getHosts: () => readonly HostInfo[], now: () => number = Date.now): MascotLive {
  const store = needsYou();
  const [changedAt, setChangedAt] = createSignal(now());
  const unread = createMemo(() => store.list().filter((i) => !store.isRead(i.id)));
  const waiting = createMemo(() => unread().filter((i) => i.group === 'waiting').length);
  const finished = createMemo(() => unread().filter((i) => i.group === 'finished').length);
  const running = createMemo(() => runningAgents(getHosts()));
  // Anything changing wakes the crab up.
  createEffect(on([() => unread().map((i) => i.id).join('|'), running], () => setChangedAt(now()), { defer: true }));
  const inputs = createMemo<MascotInputs>(() => {
    // The shared 30 s clock (paused while the app is hidden) re-evaluates "quiet for a while".
    clockNow(30_000);
    const hosts = getHosts();
    return { hosts: hosts.length, offlineHosts: offlineHosts(hosts), waiting: waiting(), finished: finished(), running: running(), quietMs: now() - changedAt() };
  });
  return {
    inputs,
    state: createMemo(() => mascotState(inputs())),
    count: () => store.unreadCount(),
    poke: () => setChangedAt(now()),
  };
}

let shared: MascotLive | undefined;
/** The one shared live mascot state; the first caller's host accessor wins. */
export function mascotLive(getHosts: () => readonly HostInfo[]): MascotLive {
  if (!shared) shared = createRoot(() => create(getHosts));
  return shared;
}

/** Tests: a fresh instance with an injectable clock. */
export function createMascotLiveForTest(getHosts: () => readonly HostInfo[], now: () => number): MascotLive {
  return createRoot(() => create(getHosts, now));
}
