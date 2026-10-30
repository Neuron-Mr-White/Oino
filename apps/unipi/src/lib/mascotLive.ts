/**
 * The mascot's live state (UNI-200): one reactive source for the floating
 * crab, the desktop header crab, the desktop mascot window and the Android
 * overlay — derived from the "needs you" store + every host's session list
 * (the same data the bell badge and tray already use; no new poll).
 */
import { createMemo, createRoot, createSignal, on, createEffect, type Accessor } from 'solid-js';
import { needsYou } from './needsYou';
import { chatLists } from './session/list';
import type { HostInfo } from './schemas';
import { mascotState, type MascotInputs, type MascotState } from './unicrab';

export type MascotLive = { state: Accessor<MascotState>; count: Accessor<number>; inputs: Accessor<MascotInputs>; poke: () => void };

/** Sessions working across `hosts` (session cards in state `working`). */
export function runningAgents(hosts: readonly HostInfo[]): number {
  let running = 0;
  for (const host of hosts) {
    for (const card of chatLists().get(host.id)?.data?.sessions ?? []) if (card.run_state === 'working' || card.pending_work) running += 1;
  }
  return running;
}

/** Hosts whose last session list failed and that never answered. */
export function offlineHosts(hosts: readonly HostInfo[]): number {
  return hosts.filter((h) => {
    const e = chatLists().get(h.id);
    return !!e?.error && !e.data;
  }).length;
}

function create(getHosts: () => readonly HostInfo[], now: () => number = Date.now): MascotLive {
  const store = needsYou();
  const [changedAt, setChangedAt] = createSignal(now());
  const [tick, setTick] = createSignal(0);
  const unread = createMemo(() => store.list().filter((i) => !store.isRead(i.id)));
  const waiting = createMemo(() => unread().filter((i) => i.group === 'waiting').length);
  const finished = createMemo(() => unread().filter((i) => i.group === 'finished').length);
  const running = createMemo(() => runningAgents(getHosts()));
  // Anything changing wakes the crab up.
  createEffect(on([() => unread().map((i) => i.id).join('|'), running], () => setChangedAt(now()), { defer: true }));
  if (typeof setInterval !== 'undefined') setInterval(() => setTick((n) => n + 1), 30_000);
  const inputs = createMemo<MascotInputs>(() => {
    tick();
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
