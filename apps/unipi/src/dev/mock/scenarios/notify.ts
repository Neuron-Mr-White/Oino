/**
 * Notification scenarios (WP-A19):
 *
 *   needs-you  the Unicrab drawer grouped Waiting / Finished / Other across
 *              two hosts — a session that starts waiting and one that
 *              finishes while nobody watches (hub `session_changed` script)
 *   toasts     the one app-wide stack: a folded info ×2, a warning, a
 *              sticky error (info 4 s, warning 6 s, error stays)
 *   unread     sidebar unread dots: seeded ones plus a run that finishes
 *              unfocused and is marked unread by the host (Z-86)
 */
import type { MockScenario, MockHostSpec } from '../host';
import type { SessionCard } from '../../../lib/host/client';
import { genCards } from '../gen';
import { toasts } from '../../../lib/toasts';
import { setNeedsYouOpen } from '../../../components/FloatingBell';

const FOLDERS = ['unipi', 'oino', 'website'];

/** A card copy with patches (hub scripts emit the next state of a card). */
function patch(card: SessionCard, over: Partial<SessionCard>): SessionCard {
  return { ...card, ...over };
}

/** The local host's cards: `w1` runs, `w2` waits soon, one crashed, some unread. */
function localCards(): SessionCard[] {
  const base = genCards(8, { seed: 5, folders: FOLDERS, needsYou: 0, unread: 2, changes: true });
  const named = base.map((card, i) => ({
    ...card,
    name: ['Fix the queue drain', 'Nightly pages build', 'Refactor the hub store', 'Answer the trap question', 'Ship the toast stack', 'Cron: weekly recap', 'Trim the timeline', 'Pair the phone'][i] ?? card.name,
  }));
  const w1 = patch(named[0]!, { session_id: 'w1', run_state: 'working', name: 'Migrate the session store' });
  const w2 = patch(named[1]!, { session_id: 'w2', run_state: 'working', name: 'Investigate the flaky test' });
  return [w1, w2, ...named.slice(2).map((card, i) => patch(card, { session_id: `s1${String(i).padStart(2, '0')}` }))];
}

/** The hub script: `w2` starts waiting, `w1` finishes unseen (unread, Z-86). */
function hubScript(cards: () => SessionCard[]): NonNullable<MockHostSpec['hub']> {
  return [
    {
      at: 400,
      event: {
        kind: 'session_changed',
        card: patch(cards().find((c) => c.session_id === 'w2')!, { needs_you: true, last_activity: Date.now() }),
      },
    },
    {
      at: 800,
      event: {
        kind: 'session_changed',
        card: patch(cards().find((c) => c.session_id === 'w1')!, { run_state: 'idle', unread: true, last_activity: Date.now() }),
      },
    },
  ];
}

export const needsYou: MockScenario = {
  name: 'needs-you',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      get cards() {
        return localCards();
      },
      get hub() {
        return hubScript(localCards);
      },
    },
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(3, { seed: 9, prefix: 'c', folders: ['side'], needsYou: 2, unread: 1 }),
    },
  ],
  setup: () => setNeedsYouOpen(true),
};

export const toastsScenario: MockScenario = {
  name: 'toasts',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(4, { seed: 5, folders: FOLDERS }),
    },
  ],
  setup: () => {
    const stack = toasts();
    stack.push('Committed abc1234 · fix: sidebar width');
    stack.push('Committed abc1234 · fix: sidebar width'); // folds ×2
    stack.push('Connection flaky — retrying', 'warning');
    stack.push('Export failed: disk full', 'error');
  },
};

export const unread: MockScenario = {
  name: 'unread',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      get cards() {
        return localCards();
      },
      get hub() {
        return hubScript(localCards);
      },
    },
  ],
};
