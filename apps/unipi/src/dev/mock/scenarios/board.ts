/**
 * Board scenarios (WP-A22): the kanboard screen on the `unipi-kanboard` CLI
 * seam (`board_cli` / `board_watch` answering from `dev/mock/board.ts`, the
 * same engine the tests use).
 *
 *   board-lanes      one project, every open lane populated: a running claim,
 *                    a stale one, a review report, a blocked report, chains,
 *                    labels/priorities — the lanes + card captures.
 *   board-dashboard  the Overview (dashboard) of three projects: a needs-you
 *                    inbox (review + blocked), ready work, agents running and
 *                    two weeks of throughput — captured from `setup`.
 */
import type { MockScenario } from '../host';
import type { MockBoardProject } from '../board';
import { genCards } from '../gen';
import type { Task } from '../../../lib/board/api';
import { boardUi } from '../../../lib/board/ui';
import { oin20Report, stressReport } from '../report-fixtures';

const HOST = 'this-pc';
const SLUG = 'unipi-7c1';
const NOW = Date.UTC(2026, 9, 30, 9, 0, 0);
const iso = (minutesAgo: number): string => new Date(NOW - minutesAgo * 60_000).toISOString();

let seq = 0;
function task(partial: Partial<Task> & { id: string; title: string }): Task {
  seq += 1;
  return {
    status: 'todo',
    priority: 'none',
    order: seq * 1000,
    deps: [],
    labels: [],
    created: iso(60 * 40),
    updated: iso(60 * 2),
    activity: [],
    ready: true,
    creator: 'user',
    ...partial,
  };
}

const REVIEW_BODY = [
  '## Summary',
  'Split the review request into its own pane so the queue keeps its width.',
  '',
  '## Next steps',
  '- [ ] Keyboard: `r` opens the same sheet',
  '- [x] Narrow shell falls back to the overlay',
].join('\n');

const BLOCKED_BODY = ['Need a product decision:', '', '- keep the relay fallback or', '- gate it behind `--relay` only'].join('\n');

const TASKS: Task[] = [
  task({ id: 'OIN-20', title: 'Superagent provider report', status: 'in_review', statusReport: { kind: 'review', text: oin20Report, actor: 'agent', at: iso(120) } }),
  task({ id: 'OIN-49', title: 'Report layout stress test', status: 'in_review', statusReport: { kind: 'review', text: stressReport, actor: 'agent', at: iso(120) } }),
  task({
    id: 'U-3',
    title: 'Board: live refresh from the watcher',
    status: 'in_progress',
    priority: 'high',
    labels: ['board', 'host'],
    body: 'One `board_changed` per project, debounced reload, no SSE.',
    run: { session: 's014', pid: 4211, host: HOST, owner: 'agent', started: iso(95) },
    ready: false,
    activity: [{ at: iso(95), actor: 'agent', session: 's014', text: 'claimed: working on the watcher' }],
  }),
  task({
    id: 'U-7',
    title: 'Stale claim: release the sandbox run',
    status: 'in_progress',
    priority: 'medium',
    labels: ['host'],
    staleness: 'stale',
    run: { session: 's009', pid: 3990, host: HOST, owner: 'agent', started: iso(60 * 26) },
    ready: false,
  }),
  task({
    id: 'U-11',
    title: 'Review: split the request pane',
    status: 'in_review',
    priority: 'high',
    labels: ['ui'],
    deps: ['U-12'],
    statusReport: { kind: 'review', text: REVIEW_BODY, actor: 'agent', at: iso(120) },
    activity: [{ at: iso(120), actor: 'agent', session: 's011', text: 'finished: Split the review request into its own pane so the queue keeps its width.' }],
    ready: true,
  }),
  task({
    id: 'U-14',
    title: 'Relay: fallback path needs a decision',
    status: 'blocked',
    priority: 'urgent',
    labels: ['host', 'relay'],
    statusReport: { kind: 'blocked', text: BLOCKED_BODY, actor: 'agent', at: iso(60 * 6) },
    ready: false,
    activity: [{ at: iso(60 * 6), actor: 'agent', session: 's008', text: `blocked: ${BLOCKED_BODY.split('\n')[0]}` }],
  }),
  task({
    id: 'U-12',
    title: 'Watcher: one event per project',
    status: 'todo',
    priority: 'high',
    labels: ['host'],
    body: 'Coalesce 150 ms per project, skip our own writes.',
  }),
  task({
    id: 'U-15',
    title: 'Phone: lane pager overscroll',
    status: 'todo',
    priority: 'medium',
    labels: ['ui', 'phone'],
    body: 'Rubber-band only on the last page.',
  }),
  task({
    id: 'U-9',
    title: 'Card menu: one-tap priority',
    status: 'todo',
    priority: 'low',
    labels: ['ui'],
    deps: ['U-14'],
  }),
  task({
    id: 'U-16',
    title: 'Docs: Board calls the CLI',
    status: 'todo',
    priority: 'none',
    labels: ['docs'],
    deps: ['U-3'],
    ready: false,
  }),
  task({
    id: 'U-2',
    title: 'CLI: `--json` on every subcommand',
    status: 'done',
    priority: 'high',
    labels: ['cli'],
    activity: [{ at: iso(60 * 20), actor: 'agent', session: 's003', text: 'finished: JSON on stdout, contract errors on stderr' }],
  }),
  task({
    id: 'U-6',
    title: 'Host: allow-list the board argv',
    status: 'done',
    priority: 'medium',
    labels: ['host'],
  }),
  task({
    id: 'U-1',
    title: 'Spike: drop the daemon',
    status: 'done',
    priority: 'none',
    labels: ['spike'],
  }),
  task({ id: 'U-4', title: 'Old lane strip idea', status: 'cancelled', priority: 'low', labels: ['ui'] }),
  task({ id: 'U-5', title: 'Archived: herdr board link', status: 'archived', priority: 'none', labels: ['ui'] }),
  task({ id: 'U-8', title: 'Backlog: per-lane WIP limits', status: 'backlog', priority: 'low', labels: ['ui'] }),
  task({ id: 'U-10', title: 'Backlog: print the board as markdown', status: 'backlog', priority: 'none', labels: ['cli'] }),
];

const SECOND: MockBoardProject = {
  slug: 'website-3a',
  name: 'website',
  tasks: [
    task({
      id: 'W-2',
      title: 'Pricing page copy review',
      status: 'in_review',
      priority: 'medium',
      labels: ['content'],
      statusReport: { kind: 'review', text: '## Summary\nShorter hero, one CTA.', actor: 'agent', at: iso(60 * 9) },
      activity: [{ at: iso(60 * 9), actor: 'agent', session: 's021', text: 'finished: Shorter hero, one CTA.' }],
    }),
    task({ id: 'W-4', title: 'Changelog: October', status: 'todo', priority: 'low', labels: ['content'] }),
    task({ id: 'W-5', title: 'Fix the docs nav on mobile', status: 'in_progress', priority: 'high', labels: ['bug'], run: { session: 's022', pid: 5102, host: HOST, owner: 'agent', started: iso(35) } }),
    task({ id: 'W-1', title: 'Move to the new font', status: 'done', priority: 'none', labels: ['design'] }),
  ],
};

const BOARD: MockBoardProject[] = [
  { slug: SLUG, name: 'unipi', tasks: TASKS },
  SECOND,
  {
    slug: 'oino-91f',
    name: 'oino',
    tasks: [
      task({ id: 'O-1', title: 'Session protocol v2 handshake', status: 'done', priority: 'high', labels: ['core'] }),
      task({ id: 'O-3', title: 'Compaction: keep the tail', status: 'todo', priority: 'medium', labels: ['core'] }),
    ],
  },
];

export const boardLanes: MockScenario = {
  name: 'board-lanes',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(4, { seed: 41, folders: ['unipi'], changes: true }),
      board: BOARD,
    },
  ],
  setup: () => {
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: HOST, projects: { [HOST]: SLUG } }));
  },
};

export const boardDashboard: MockScenario = {
  name: 'board-dashboard',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(4, { seed: 41, folders: ['unipi'], changes: true }),
      board: BOARD,
    },
  ],
  setup: () => {
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: HOST, projects: { [HOST]: SLUG } }));
    boardUi().setOverview(true);
  },
};
