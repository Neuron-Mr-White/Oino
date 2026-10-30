/**
 * UNI-202: remembers the last Steer/Queue choice per chat (host+session) so the
 * inline choice popover can highlight a sensible default next time. Purely
 * cosmetic — the user still has to tap one of the two pills; this just
 * decides which one looks "picked" first.
 */
export type SendChoice = 'steer' | 'after';

const key = (hostId: string, session: string) => `unipi.chat.sendChoice.${hostId}.${session}`;

export function lastSendChoice(hostId: string, session: string): SendChoice {
  try {
    const raw = localStorage.getItem(key(hostId, session));
    return raw === 'after' ? 'after' : 'steer';
  } catch {
    return 'steer';
  }
}

export function rememberSendChoice(hostId: string, session: string, choice: SendChoice): void {
  try {
    localStorage.setItem(key(hostId, session), choice);
  } catch {
    // storage full/unavailable: just not remembered next time.
  }
}
