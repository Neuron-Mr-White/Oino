/**
 * WP-A5 scenarios: the session row context menu, the Archived list and the
 * new-chat draft (Z-13/Z-15/Z-16/Z-19/Z-20).
 *
 *   session-menu   a few rows in every state so a right-click capture has
 *                  working / needs-you / pinned / cron rows to aim at.
 *   archived       one live project with a couple of sessions and a trash of
 *                  three archived ones (restore / delete / delete-all).
 *   draft          the `/new/:host` composer: recent cwds + fs roots chips.
 */
import type { MockScenario } from '../host';

const base = (over: Partial<MockScenario['hosts'][number]>): MockScenario['hosts'][number] => ({
  id: 'this-pc',
  name: 'workstation',
  transport: 'local',
  conn: { state: 'connected' },
  ...over,
});

export const sessionMenu: MockScenario = {
  name: 'session-menu',
  hosts: [
    base({
      cards: [
        { session_id: 'triage', name: 'Triage failing tests', cwd: '/home/you/p/oino', file: '/home/you/.oino/s/triage.jsonl', run_state: 'idle', updated_at: Date.now() - 4 * 60_000, message_count: 24 },
        { session_id: 'port', name: 'Port the sidebar to Solid', cwd: '/home/you/p/oino', run_state: 'working', updated_at: Date.now() - 40_000, message_count: 61 },
        { session_id: 'site', name: null, cwd: '/home/you/p/website', run_state: 'idle', updated_at: Date.now() - 3 * 3600_000, message_count: 7 },
        { session_id: 'digest', name: 'Nightly digest', cwd: '/home/you/p/oino', run_state: 'idle', source: 'cron', updated_at: Date.now() - 9 * 3600_000, message_count: 3 },
        { session_id: 'pinned', name: 'Release checklist', cwd: '/home/you/p/oino', run_state: 'idle', pinned: true, updated_at: Date.now() - 26 * 3600_000, message_count: 12 },
      ],
      fs: { '/home/you/p/oino': { kind: 'dir', children: {} }, '/home/you/p/website': { kind: 'dir', children: {} } },
    }),
  ],
};

export const archived: MockScenario = {
  name: 'archived',
  hosts: [
    base({
      cards: [
        { session_id: 'live-1', name: 'Triage failing tests', cwd: '/home/you/p/oino', run_state: 'working', updated_at: Date.now() - 60_000, message_count: 61 },
        { session_id: 'live-2', name: null, cwd: '/home/you/p/oino', run_state: 'idle', updated_at: Date.now() - 30 * 60_000, message_count: 9 },
      ],
      archived: [
        { session_id: 'old-api', name: 'Old API sketch', cwd: '/home/you/p/oino', run_state: 'idle', updated_at: Date.now() - 6 * 24 * 3600_000, message_count: 18 },
        { session_id: 'spike', name: 'Casing spike', cwd: '/home/you/p/spike', run_state: 'idle', updated_at: Date.now() - 11 * 24 * 3600_000, message_count: 44 },
        { session_id: 'scratch', name: null, cwd: '/home/you/p/scratch', run_state: 'crashed', updated_at: Date.now() - 30 * 24 * 3600_000, message_count: 2 },
      ],
      fs: { '/home/you/p/oino': { kind: 'dir', children: {} }, '/home/you/p/spike': { kind: 'dir', children: {} }, '/home/you/p/scratch': { kind: 'dir', children: {} } },
    }),
  ],
};

export const draft: MockScenario = {
  name: 'draft',
  hosts: [
    base({
      cards: [
        { session_id: 'a', name: 'Triage failing tests', cwd: '/home/you/p/oino', run_state: 'idle', updated_at: Date.now() - 4 * 60_000, message_count: 24 },
        { session_id: 'b', name: null, cwd: '/home/you/p/website', run_state: 'idle', updated_at: Date.now() - 3600_000, message_count: 7 },
      ],
      fs: {
        '/home/you/p/oino': { kind: 'dir', children: {} },
        '/home/you/p/website': { kind: 'dir', children: {} },
        '/home/you/p/agent37': { kind: 'dir', children: {} },
      },
    }),
  ],
};
