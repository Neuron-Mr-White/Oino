/**
 * UNI-238 (A7) "Keep available when UniPi is closed": user decision 3 — it
 * turns ON by default right after the FIRST phone pairs, and we tell the user
 * in the pair dialog's success state (they can turn it off).
 *
 * The systemd unit itself is installed by `always_on_set` in the Rust core;
 * this module is the app-side decision: "has this PC been offered the default
 * yet?" (a marker in the app's own settings, never the host's data dir) and
 * the note to show next to the "Paired with …" line.
 */
import { call } from './ipc';
import { z } from 'zod';
import { can } from './platform';

/** localStorage key: the first pairing has already been offered the default. */
const OFFERED_KEY = 'unipi.keepAvailable.offered';

const AlwaysOnSchemaObject = z.object({
  installed: z.boolean(),
  running: z.boolean(),
  bin: z.string().nullish(),
  linger: z.boolean().nullish(),
  lingerHint: z.string().nullish(),
  lingerCommand: z.string().nullish(),
});
export const AlwaysOnSchema = AlwaysOnSchemaObject;
export type AlwaysOn = z.infer<typeof AlwaysOnSchemaObject>;

export async function alwaysOn(): Promise<AlwaysOn | undefined> {
  const r = await call('always_on_state', undefined, AlwaysOnSchema);
  return r.ok ? r.value : undefined;
}

/**
 * Turns the service on (installing the unit) — once, right after the first
 * phone pairs. Best effort: a host without the op, or a PC without systemd,
 * just leaves the feature off (the Settings card still offers it).
 */
export async function offerKeepAvailable(): Promise<'on' | 'already' | 'failed' | 'unsupported'> {
  if (!can('localSharing')) return 'unsupported';
  let offered = false;
  try {
    offered = localStorage.getItem(OFFERED_KEY) === '1';
  } catch {
    offered = false;
  }
  if (offered) return 'already';
  try {
    localStorage.setItem(OFFERED_KEY, '1');
  } catch {
    // Private mode: still do it once for this session.
  }
  const r = await call('always_on_set', { on: true }, AlwaysOnSchema);
  return r.ok ? 'on' : 'failed';
}

/** For tests: forget that the offer was made. */
export function resetKeepAvailableOffer(): void {
  try {
    localStorage.removeItem(OFFERED_KEY);
  } catch {
    // ignore
  }
}

/** The pair-dialog success note (decision 3: tell the user, they can opt out). */
export function keepAvailableNote(state: AlwaysOn | undefined): string | undefined {
  if (!state?.installed) return undefined;
  if (state.linger === false) {
    return 'It stays available when you close Unipi — a background service keeps the host running. To keep it running after you log out too, allow it once in Settings → This computer.';
  }
  return 'It stays available when you close Unipi — a background service keeps the host running, even after a restart.';
}
