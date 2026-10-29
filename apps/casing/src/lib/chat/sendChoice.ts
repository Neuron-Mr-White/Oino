/**
 * UNI-202: remembers the last Steer/Queue choice per chat (host+pid) so the
 * inline choice popover can highlight a sensible default next time. Purely
 * cosmetic — the user still has to tap one of the two pills; this just
 * decides which one looks "picked" first.
 */
export type SendChoice = 'steer' | 'after';

const key = (hostId: string, pid: number) => `unipi.chat.sendChoice.${hostId}.${pid}`;

export function lastSendChoice(hostId: string, pid: number): SendChoice {
  try {
    const raw = localStorage.getItem(key(hostId, pid));
    return raw === 'after' ? 'after' : 'steer';
  } catch {
    return 'steer';
  }
}

export function rememberSendChoice(hostId: string, pid: number, choice: SendChoice): void {
  try {
    localStorage.setItem(key(hostId, pid), choice);
  } catch {
    // storage full/unavailable: just not remembered next time.
  }
}
