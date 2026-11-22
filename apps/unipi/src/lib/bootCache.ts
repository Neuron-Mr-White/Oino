/**
 * w2-ui2 T1: the last host list and session list, kept in localStorage so the first render of
 * the sidebar needs no IPC round trip. The launch used to wait for `list_hosts` and `sessions_list`
 * (their replies queue behind the module-evaluation stall) and then render the rows in a second
 * long task; with this the rows are part of the first mount and the live lists only reconcile them.
 *
 * Stale by design (stale-while-revalidate): a card that no longer exists disappears when the
 * real `sessions_list` answers, a new one appears. The hub keeps `loaded` false until then, so no
 * "missing session" or "no sessions yet" decision is ever taken from the cache. Writes are off the
 * render path (one timer per key, a second after the last change).
 */
import type { SessionCard } from './host/client';
import { HostInfoSchema, type HostInfo } from './schemas';
import { z } from 'zod';

/** Off under vitest: tests share one jsdom localStorage and must not see each other's lists. */
export const BOOT_CACHE_ON = import.meta.env?.MODE !== 'test';
const HOSTS_KEY = 'unipi.boot.hosts.v1';
const cardsKey = (hostId: string) => `unipi.boot.cards.v1.${hostId}`;
/** Cards kept per host: more than the sidebar shows before "Show more", bounded so the write stays small. */
export const BOOT_CARDS_MAX = 150;
/** Quiet time before a changed list is written. */
export const BOOT_WRITE_MS = 1000;

const HostsSchema = z.array(HostInfoSchema);

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : undefined;
  } catch {
    return undefined;
  }
}

/** The last `list_hosts` result, or undefined (first launch, bad data). */
export function readBootHosts(): HostInfo[] | undefined {
  const parsed = HostsSchema.safeParse(read(HOSTS_KEY));
  return parsed.success && parsed.data.length > 0 ? parsed.data : undefined;
}

/** The last session list of one host, newest first, or undefined. */
export function readBootCards(hostId: string): SessionCard[] | undefined {
  const value = read(cardsKey(hostId));
  if (!Array.isArray(value)) return undefined;
  const cards = value.filter((c): c is SessionCard => !!c && typeof c === 'object' && typeof (c as SessionCard).session_id === 'string');
  return cards.length > 0 ? cards : undefined;
}

const timers = new Map<string, ReturnType<typeof setTimeout>>();
function writeSoon(key: string, produce: () => unknown): void {
  const pending = timers.get(key);
  if (pending !== undefined) clearTimeout(pending);
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      try {
        localStorage.setItem(key, JSON.stringify(produce()));
      } catch {
        /* storage full or unavailable: the next launch just starts cold */
      }
    }, BOOT_WRITE_MS),
  );
}

/** The pairing tickets are not kept (nothing in the UI reads them; the live `list_hosts` brings them back). */
export function writeBootHosts(hosts: () => readonly HostInfo[]): void {
  writeSoon(HOSTS_KEY, () => hosts().map((h) => ({ ...h, ticket: '-' })));
}

/** `cards()` is read when the timer fires, so a burst of changes writes once with the latest list. */
export function writeBootCards(hostId: string, cards: () => readonly SessionCard[]): void {
  writeSoon(cardsKey(hostId), () =>
    [...cards()]
      .sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0))
      .slice(0, BOOT_CARDS_MAX),
  );
}
