/**
 * The `local` alias (This computer), resolved once, centrally. Routes and
 * settings say `local` (the first-run route `/new/local`, Settings' host
 * commands); the desktop core knows This computer only by its real id (its
 * `list_hosts` row with `transport: "local"`). Two names for one PC gave two
 * hubs and two session registries — and a hub created mid-navigation, which
 * crashed Solid's cleanup (`null is not an object (e.owned[t])`).
 *
 * `HostsProvider` records the real id when the hosts load; the Tauri client
 * resolves the alias on the wire (`wireHostId`); the routes redirect
 * `/new/local` and `/s/local/…` to the real id (main.tsx).
 */
import { createSignal } from 'solid-js';
import { LOCAL_HOST } from './client';

const [localId, setLocalId] = createSignal<string | undefined>();

/** This computer's real host id, once known. */
export const localHostId = localId;

export function setLocalHostId(id: string | undefined): void {
  if (id && id !== localId()) setLocalId(id);
}

/** `local` → the real id when known; any other id unchanged. */
export function resolveHostId(id: string): string {
  return id === LOCAL_HOST ? (localId() ?? id) : id;
}

/** The `transport: "local"` row of a `list_hosts` answer. */
export function localIdOf(hosts: ReadonlyArray<{ id: string; transport?: string }>): string | undefined {
  return hosts.find((h) => h.transport === 'local')?.id;
}

/** Tests. */
export function resetLocalHostId(): void {
  setLocalId(undefined);
}
