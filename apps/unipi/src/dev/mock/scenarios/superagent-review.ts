/**
 * Superagent with a real review queue (OIN-40): seven tickets in review across
 * three Kanboard projects, answered by the in-memory board, so opening one from
 * the Review accordion, Mark done / Rework and Prev / Next work end to end and
 * the Review list shrinks as tickets leave. `?scenario=superagent-review`.
 */
import type { MockScenario } from '../host';
import type { MockBoardProject } from '../board';
import type { Task } from '../../../lib/board/api';
import { superagentLeads } from './superagent-leads';

const base = superagentLeads.hosts![0]!;
const { board_cli: _board, ...ops } = base.ops!;
const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
let seq = 0;
function ticket(id: string, title: string, summary: string, labels: string[] = []): Task {
  seq += 1;
  return {
    id, title, status: 'in_review', priority: 'medium', order: seq * 1000, deps: [], labels, created: ago(60 * 30), updated: ago(40), activity: [{ at: ago(40), actor: 'agent', session: 'lead-hp', text: `finished: ${summary}` }],
    ready: true, creator: 'user',
    statusReport: { kind: 'review', text: `## Summary\n${summary}\n\n## Checks\n- [x] Unit tests pass\n- [x] Typecheck is clean`, actor: 'agent', at: ago(40) },
  } as Task;
}
const BOARD: MockBoardProject[] = [
  { slug: 'heartpick', name: 'Heartpick', prefix: 'HP', tasks: [
    ticket('HP-3', 'Retry banner flickers', 'Debounced the retry banner so it only shows after 400 ms offline.', ['sync']),
    ticket('HP-5', 'Offline queue loses order', 'The offline queue now replays in the order edits were made.', ['sync']),
    ticket('HP-8', 'Duplicate rows on import', 'Imports de-duplicate on the stable row id before inserting.', ['import']),
  ] },
  { slug: 'oino', name: 'Oino', prefix: 'OIN', tasks: [
    ticket('OIN-12', 'Autonomy controls and approvals', 'Added the three-level autonomy chip and the approval card.', ['ui']),
    ticket('OIN-17', 'Lead presets', 'Presets carry model, thinking, autonomy and mode.', ['superagent']),
  ] },
  { slug: 'notes', name: 'Field notes', prefix: 'NOT', tasks: [
    ticket('NOT-2', 'Tag filter in the sidebar', 'Tags filter the note list from the sidebar.', ['ui']),
    ticket('NOT-4', 'Export a notebook as markdown', 'One markdown file per note, zipped.', ['export']),
  ] },
];
const sameAs = (status: unknown) => status as { review: unknown[] };

export const superagentReview: MockScenario = {
  name: 'superagent-review',
  hosts: [{
    ...base,
    board: BOARD,
    ops: {
      ...ops,
      'superagent.status': (a, host) => {
        const status = sameAs((ops['superagent.status'] as (a: Record<string, unknown>, h: typeof host) => unknown)(a, host));
        const review = BOARD.flatMap((p) => host.board.tasks(p.slug).filter((t) => t.status === 'in_review').map((t) => ({ projectId: p.slug, taskId: t.id, status: 'in_review' })));
        return { ...status, review };
      },
    },
  }],
  setup() {
    location.hash = '#/superagent?session=superagent-chat';
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'heartpick' } }));
  },
};
