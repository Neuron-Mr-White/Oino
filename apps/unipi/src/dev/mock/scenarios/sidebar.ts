/**
 * Sidebar scenarios (WP-A4): the sessions list under load and in every row
 * state.
 *
 *   sidebar-many     3 hosts, 12 projects, 300 sessions (incl. one 80-session
 *                    project that renders windowed once expanded) — the ≤ 16 ms
 *                    bench case and the paging/virtualization captures.
 *   sidebar-states   every Z-12 row state: working, needs-you, crashed,
 *                    taken-over, idle, pinned, unread, +N −M, cron badge,
 *                    long title (fade mask).
 */
import type { MockScenario } from '../host';
import { genCards } from '../gen';

const FOLDERS = ['unipi', 'oino', 'website', 'docs-site', 'mobile', 'cli', 'agent', 'sandbox', 'relay', 'models', 'ui', 'bench'];

/** 3 hosts, 12 projects, 300 sessions; the first project of the local host holds 80. */
export const sidebarMany: MockScenario = {
  name: 'sidebar-many',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: [
        ...genCards(80, { seed: 11, folders: [FOLDERS[0]!], pinned: 3, unread: 5, changes: true, cron: true, needsYou: 17, longTitles: true }),
        ...genCards(140, { seed: 12, prefix: 'm', folders: FOLDERS.slice(1, 7)!, changes: true, needsYou: 31 }),
      ],
    },
    {
      id: 'pc-coffee',
      name: 'coffee',
      conn: { state: 'connected' },
      cards: genCards(50, { seed: 21, prefix: 'c', root: '/home/me/work', folders: FOLDERS.slice(7, 10)!, unread: 4, changes: true }),
    },
    {
      id: 'pc-desmond',
      name: 'desmond',
      conn: { state: 'connected' },
      cards: genCards(30, { seed: 31, prefix: 'd', root: '/Users/me/src', folders: FOLDERS.slice(10, 12)!, changes: true, cron: true }),
    },
  ],
};

/** Every row state once, on two hosts. */
export const sidebarStates: MockScenario = {
  name: 'sidebar-states',
  hosts: [
    {
      id: 'this-pc',
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(9, { seed: 5, folders: ['unipi', 'oino', 'website'], pinned: 2, unread: 2, changes: true, cron: true, needsYou: 4, longTitles: true }),
    },
    {
      id: 'pc-offline',
      name: 'old-laptop',
      conn: { state: 'offline', message: 'old-laptop is not reachable' },
      cards: genCards(2, { seed: 6, folders: ['side'] }),
    },
  ],
};
