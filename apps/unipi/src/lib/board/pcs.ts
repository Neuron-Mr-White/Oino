/**
 * PCs the Board can talk to (v2): every host from `useHosts()` — in the app
 * that is "This computer" plus the paired PCs; under the capture harness the
 * mock world's hosts. Availability is checked per PC through the board
 * transport (one `status` CLI call) and cached for the session. The v1
 * synthetic web PC (a kanboard daemon serving the page) is gone with the
 * daemon.
 */
import { createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';
import { boardTransport, type BoardAvailability } from './transport';

export type Pc = { id: string; name: string };

/** The Board's PC list: the hosts the app knows (captures: the mock's). */
export function boardPcs(hosts: readonly { id: string; name: string }[]): Pc[] {
  return hosts.map((host) => ({ id: host.id, name: host.name }));
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

/** One-line text for the PC chip's availability state. */
export function availabilityText(availability: Availability | undefined): string {
  switch (availability?.state) {
    case 'ok':
      return 'Board ready';
    case 'offline':
      return 'Offline';
    case 'not_installed':
      return availability.hint || 'unipi-kanboard is not installed on this PC';
    case 'error':
      return availability.message;
    case 'checking':
    case undefined:
      return 'Checking…';
  }
}
