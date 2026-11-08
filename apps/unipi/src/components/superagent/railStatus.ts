import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import { can } from '../../lib/platform';
import { superagentApi } from './api';
import { countNeeds } from './needs';

/** What the sidebar rail shows for Superagent. */
export type RailSummary = { needs: number; leads: number };

export const RAIL_POLL_MS = 10_000;

// One shared poller for the whole app (the page runs its own, faster one): it runs while at least one indicator is mounted.
const [summary, setSummary] = createSignal<RailSummary | undefined>();
let refs = 0;
let host: string | undefined;
let timer: ReturnType<typeof setInterval> | undefined;
let generation = 0;

function stop() {
  generation++;
  if (timer !== undefined) clearInterval(timer);
  timer = undefined;
  host = undefined;
}

function start(next: string | undefined) {
  stop();
  setSummary(undefined);
  host = next;
  if (!next || !can('chat')) return;
  const gen = generation;
  const tick = async () => {
    try {
      const result = await superagentApi.status(next);
      if (gen !== generation) return;
      if (result.ok) { setSummary({ needs: countNeeds(result.value.needsYou), leads: result.value.leads.length }); return; }
      setSummary(undefined);
      // An older host without Superagent: nothing to show, nothing to poll.
      if (String(result.error.code) === 'E_UNKNOWN_OP' && timer !== undefined) { clearInterval(timer); timer = undefined; }
    } catch { if (gen === generation) setSummary(undefined); }
  };
  void tick();
  timer = setInterval(() => void tick(), RAIL_POLL_MS);
}

/** The selected host's Superagent summary, polled every 10 s while a component uses this. `undefined` = idle-unknown: show nothing. */
export function useRailStatus(hostId: () => string | undefined): () => RailSummary | undefined {
  createEffect(on(hostId, (id) => {
    refs++;
    if (refs === 1 || id !== host) start(id);
    onCleanup(() => { refs--; if (refs === 0) { stop(); setSummary(undefined); } });
  }));
  return summary;
}
