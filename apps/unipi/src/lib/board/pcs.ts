/**
 * PCs the Board can talk to (UNI-130): every paired host from
 * `useHosts()`, or — in web mode (lib/platform.ts), where there are no
 * paired hosts — one synthetic PC: the kanboard daemon serving this page
 * (same origin) or, under `vite dev`, the one behind the `/kb` proxy. Availability is checked per PC through the board
 * transport and cached for the session.
 */
import { createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';
import { boardTransport, DEV_HOST_ID, servedByDaemon, type BoardAvailability } from './transport';
import { isTauri } from '../platform';

export type Pc = { id: string; name: string; dev?: boolean };
export const DEV_PC: Pc = { id: DEV_HOST_ID, name: 'dev (vite proxy)', dev: true };

/** The synthetic web-mode PC: "This computer" when the daemon serves the page. */
export function webPc(): Pc {
  if (!servedByDaemon()) return DEV_PC;
  const host = typeof location !== 'undefined' ? location.hostname : '';
  const local = host === '' || host === 'localhost' || host === '127.0.0.1' || host === '::1' || host === '[::1]';
  return { id: DEV_HOST_ID, name: local ? 'This computer' : host, dev: true };
}

/** The Board's PC list: paired hosts, or the synthetic web PC (no iroh hosts in a browser). */
export function boardPcs(hosts: readonly { id: string; name: string }[], tauri = isTauri()): Pc[] {
  if (hosts.length > 0) return hosts.map((host) => ({ id: host.id, name: host.name }));
  return tauri ? [] : [webPc()];
}

export type Availability = BoardAvailability | { state: 'checking' };

function createAvailability() {
  const [map, setMap] = createStore<Record<string, Availability>>({});
  const inflight = new Map<string, Promise<BoardAvailability>>();
  function check(hostId: string): Promise<BoardAvailability> {
    const running = inflight.get(hostId);
    if (running) return running;
    if (!map[hostId] || map[hostId]!.state !== 'ok') setMap(hostId, { state: 'checking' });
    const promise = boardTransport()
      .availability(hostId)
      .catch((error): BoardAvailability => ({ state: 'error', message: String((error as Error)?.message ?? error) }))
      .then((result) => {
        setMap(hostId, result);
        inflight.delete(hostId);
        return result;
      });
    inflight.set(hostId, promise);
    return promise;
  }
  return { availability: (hostId: string): Availability | undefined => map[hostId], check };
}

let shared: ReturnType<typeof createAvailability> | undefined;
export function pcAvailability() {
  if (!shared) shared = createRoot(() => createAvailability());
  return shared;
}
export function resetPcAvailability(): void {
  shared = undefined;
}

/** One-line status for a PC row. */
export function availabilityText(availability: Availability | undefined): string {
  switch (availability?.state) {
    case 'ok':
      return availability.version ? `Board ${availability.version}` : 'Board ready';
    case 'offline':
      return 'Offline';
    case 'not_installed':
      return availability.hint || 'unipi-host is not installed on this PC';
    case 'error':
      return availability.message;
    case 'checking':
    case undefined:
      return 'Checking…';
  }
}
