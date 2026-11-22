/**
 * Git-pane scenarios (WP-A15):
 *
 *   git-pane       the status pane's Git section — Unstaged, the unipi
 *                  grouping, +/- counts, the commit composer (desktop pane +
 *                  phone layer)
 *   git-last-turn  the same repo with the "Last turn" source selected (the
 *                  last turn's write/edit files, from the turns index)
 *   git-history    the History tab (log with ref pills, Load more)
 *   git-pane-many  2 000 changed files (the virtualization row of the
 *                  acceptance: scroll frames at 60 fps)
 *
 * The repo is the project folder of the seeded session; `git_op` answers from
 * the one mock repo, so stage/commit/discard move rows in the capture.
 */
import type { MockGitFile, MockScenario, MockSession } from '../host';
import { desktopShell } from '../../../lib/layout';
import { openGit, type GitSource } from '../../../lib/git';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';

const commit = (i: number, subject: string, extra: Record<string, unknown> = {}) => ({
  hash: `${String(i).padStart(2, '0')}c0ffee${'0'.repeat(31)}`,
  subject,
  author: ['Ann', 'Ben', 'Cleo'][i % 3]!,
  time: 1_760_000_000 - i * 5400,
  ...extra,
});

const COMMITS = [
  commit(0, 'Wire the status pane to the git watcher', { refs: ['HEAD -> main', 'origin/main'] }),
  commit(1, 'Virtualize the changes list', { refs: ['tag: v2.0.0-rc.3'] }),
  commit(2, 'Session store: pin, unread and title lock'),
  commit(3, 'Composer: attachment chips and image editor'),
  commit(4, 'Host: board_watch hub events', { refs: ['origin/feat/board'] }),
  commit(5, 'Protocol: git watch request and event'),
  commit(6, 'Files: folder tree with git badges'),
  commit(7, 'Shell: routing and the side-pane tabs'),
  ...Array.from({ length: 52 }, (_, i) =>
    commit(8 + i, [
      'Tighten the diff header spacing',
      'Fix the reducer tail recompute',
      'Cache the solved layout per width',
      'Sidebar: relative times and change counts',
      'Watchdog: stop the run at the turn cap',
    ][i % 5]!),
  ),
];

const FILES: MockGitFile[] = [
  { path: 'src/components/status/GitSection.tsx', area: 'unstaged', status: 'M', add: 96, del: 12, orig: 'src/components/git/GitView.tsx' },
  { path: 'src/lib/git.ts', area: 'unstaged', status: 'M', add: 44, del: 6 },
  { path: 'src/components/git/git.css', area: 'unstaged', status: 'M', add: 31, del: 9 },
  { path: 'src/lib/session/hub.ts', area: 'unstaged', status: 'M', add: 12, del: 2 },
  { path: 'src/components/shell/AppShell.tsx', area: 'unstaged', status: 'M', add: 7, del: 1 },
  { path: 'apps/unipi/crates/unipi-host/src/git_watch.rs', area: 'unstaged', status: 'A', add: 321, del: 0 },
  { path: 'apps/unipi/crates/unipi-host/tests/git_watch.rs', area: 'unstaged', status: 'A', add: 229, del: 0 },
  { path: 'apps/unipi/crates/unipi-proto/src/git.rs', area: 'unstaged', status: 'M', add: 38, del: 1 },
  { path: 'packages/protocol/src/git.ts', area: 'unstaged', status: 'M', add: 28, del: 0 },
  { path: 'assets/logo.png', area: 'unstaged', status: 'M', binary: true },
  { path: 'src/components/git/BranchSheet.tsx', area: 'staged', status: 'A', add: 134, del: 0 },
  { path: 'src/components/git/GitChanges.tsx', area: 'staged', status: 'A', add: 207, del: 0 },
  { path: 'docs/dev/v2/specs/app.md', area: 'staged', status: 'M', add: 18, del: 4 },
  { path: 'src/dev/mock/scenarios/git.ts', area: 'untracked', status: '?', add: 64, del: 0 },
  { path: 'src/components/status/StatusPane.tsx', area: 'untracked', status: '?', add: 96, del: 0 },
  { path: 'notes/git-pane.md', area: 'untracked', status: '?', add: 31, del: 0 },
  { path: 'scratch/diff.txt', area: 'untracked', status: '?', add: 9, del: 0 },
  { path: 'src/lib/panes/tabs.ts', area: 'conflict', status: 'U', add: 6, del: 6 },
];

const MANY: MockGitFile[] = Array.from({ length: 2000 }, (_, i) => {
  const dir = ['src/components', 'src/lib', 'apps/unipi/crates/unipi-host/src', 'packages/protocol/src'][i % 4]!;
  return {
    path: `${dir}/generated/file-${String(i).padStart(4, '0')}.ts`,
    area: i % 9 === 0 ? 'staged' : i % 7 === 0 ? 'untracked' : 'unstaged',
    status: i % 7 === 0 ? '?' : 'M',
    add: 3 + (i % 40),
    del: i % 11,
  } as MockGitFile;
});

const ENTRIES = [
  { type: 'message', id: 'e1', message: { role: 'user', content: 'Re-home the Git view into the status pane and watch the repo.', timestamp: 1_759_999_000 } },
  {
    type: 'message',
    id: 'e2',
    message: {
      role: 'assistant',
      stopReason: 'toolUse',
      timestamp: 1_759_999_100,
      content: [{ type: 'toolCall', id: 't1', name: 'edit', arguments: { path: 'src/lib/git.ts', edits: [{ oldText: 'export const LOG_PAGE = 40;', newText: 'export const LOG_PAGE = 50;' }] } }],
    },
  },
  { type: 'message', id: 'e3', message: { role: 'toolResult', toolCallId: 't1', toolName: 'edit', isError: false, content: [{ type: 'text', text: 'Successfully replaced 1 block in src/lib/git.ts.' }] } },
  {
    type: 'message',
    id: 'e4',
    message: {
      role: 'assistant',
      stopReason: 'toolUse',
      timestamp: 1_759_999_200,
      content: [{ type: 'toolCall', id: 't2', name: 'edit', arguments: { file_path: 'docs/dev/v2/specs/app.md', edits: [{ oldText: 'Z-98 auto-refresh', newText: 'Z-98 auto-refresh (host watcher)' }] } }],
    },
  },
  { type: 'message', id: 'e5', message: { role: 'toolResult', toolCallId: 't2', toolName: 'edit', isError: false, content: [{ type: 'text', text: 'Successfully replaced 1 block in docs/dev/v2/specs/app.md.' }] } },
  { type: 'message', id: 'e6', message: { role: 'assistant', stopReason: 'stop', timestamp: 1_759_999_300, content: [{ type: 'text', text: 'The Git section now lives in the status pane and refreshes on the host watcher.' }] } },
];

const repo = {
  repo: true,
  root: CWD,
  branch: 'main',
  head: 'a1b2c3d',
  upstream: 'origin/main',
  ahead: 2,
  behind: 1,
  remotes: ['origin'],
  stashes: 1,
  files: FILES,
  base: 'origin/main',
  commits: COMMITS,
  diffs: {
    'src/lib/git.ts':
      'diff --git a/src/lib/git.ts b/src/lib/git.ts\n--- a/src/lib/git.ts\n+++ b/src/lib/git.ts\n@@ -18,3 +18,4 @@ export type GitSource = \'unstaged\' | \'staged\' | \'branch\' | \'turn\';\n export const GIT_SOURCES = [\n   { id: \'unstaged\', label: \'Unstaged\' },\n   { id: \'staged\', label: \'Staged\' },\n+  { id: \'branch\', label: \'Branch\' },\n   { id: \'turn\', label: \'Last turn\' },\n ];\n',
  },
};

function session(): MockSession {
  return {
    snapshot: {
      session_id: 's001',
      reset: true,
      seq: 6,
      state: { sessionId: 's001', isStreaming: false, isCompacting: false, model: { provider: 'anthropic', id: 'claude-sonnet-4-5' }, thinkingLevel: 'medium', messageCount: ENTRIES.length, pendingMessageCount: 0 },
      entries: ENTRIES,
      leaf_id: 'e6',
      commands: [],
      ui: { scope: 'session', sessionId: 's001', rev: 0, entries: [], dialogs: [] },
      topics: [],
    },
    events: [],
  };
}

const card = (id: string, name: string, updated: number) => ({
  session_id: id,
  name,
  cwd: CWD,
  run_state: 'idle' as const,
  updated_at: updated,
  message_count: 6,
  changes: { add: 96, del: 12 },
});

const hosts = (files: MockGitFile[] = FILES) => [
  {
    id: HOST,
    name: 'workstation',
    transport: 'local' as const,
    conn: { state: 'connected' as const },
    cards: [card('s001', 'Re-home the Git view into the status pane', 1_760_000_000), card('s002', 'Profile the board on a cold cache', 1_759_900_000)],
    sessions: { s001: session() },
    git: { ...repo, files },
  },
];

type GitParams = { path: string; source?: GitSource; tab?: 'changes' | 'history' };

/** Seeds the desktop side pane on the `git` tab (+ the last route). */
function seed(params: GitParams): void {
  localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 480, bottomOpen: false, bottomH: 220 }));
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 's001' }));
  localStorage.setItem(`unipi.panes:${HOST}|${CWD}`, JSON.stringify({
    side: { tabs: [{ id: 'tg', kind: 'git', params }], active: { s001: 'tg' }, closed: [] },
    bottom: { tabs: [], active: {}, closed: [] },
  }));
}

/** The phone has no side pane: the same section opens as the §3.9 layer. */
function openOnPhone(params: GitParams): void {
  setTimeout(() => {
    if (desktopShell()) return;
    openGit({
      hostId: HOST,
      path: params.path,
      sessionId: 's001',
      from: 'chat',
      ...(params.source ? { source: params.source } : {}),
      ...(params.tab ? { tab: params.tab } : {}),
    });
  }, 250);
}

function scenario(name: string, params: GitParams, files: MockGitFile[] = FILES): MockScenario {
  return {
    name,
    hosts: hosts(files),
    setup: () => {
      seed(params);
      openOnPhone(params);
    },
  };
}

export const gitPane: MockScenario = scenario('git-pane', { path: CWD });
export const gitLastTurn: MockScenario = scenario('git-last-turn', { path: CWD, source: 'turn' });
export const gitHistory: MockScenario = scenario('git-history', { path: CWD, tab: 'history' });
export const gitPaneMany: MockScenario = scenario('git-pane-many', { path: CWD }, MANY);
