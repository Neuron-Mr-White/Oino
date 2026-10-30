/**
 * WP-A18 capture scenarios:
 *
 * - `cmdk-empty`   the command center on Cmd/Ctrl+K: Recent sessions (3) +
 *                  grouped commands (≤ 3 + "Show more"), history chips after a
 *                  search. Press Ctrl+K.
 * - `cmdk-files`   Ctrl+P → Files scope: `fs_search` over the project tree.
 * - `find-bar`     a 40-turn session; Ctrl+F, type `tally`: bar centred over
 *                  the conversation, marks in the timeline, wrap with Enter.
 * - `shortcuts-list` Ctrl+/: the registry table incl. the rebound extension
 *                  chords (Alt+B subagents, intercepted Shift+↓ tray).
 */
import type { MockScenario } from '../host';
import { genCards, genEntries, genFsTree, genSession } from '../gen';

const LOCAL = 'this-pc';
const CWD = '/home/you/projects/unipi';

function commandSession() {
  // A few turns whose texts repeat `tally` so find has matches everywhere.
  const entries = genEntries({ seed: 11, turns: 40, toolsPerTurn: 1, thinking: true, cwd: CWD });
  let n = 0;
  for (const entry of entries) {
    const message = entry.message as { role?: string; content?: Array<{ type: string; text?: string }> } | undefined;
    if (message?.role === 'user') message.content = [{ type: 'text', text: `Please tally the results of run ${++n}` }];
    if (message?.role === 'assistant' && Array.isArray(message.content)) {
      for (const part of message.content) if (part.type === 'text') part.text = 'The tally is ready and checked against the suite.';
    }
  }
  const mock = genSession('s001', { seed: 11, turns: 2, cwd: CWD });
  mock.snapshot.entries = entries as unknown as typeof mock.snapshot.entries;
  mock.snapshot.leaf_id = entries.at(-1)?.id ?? null;
  mock.snapshot.commands = [
    { name: '/permission', description: 'Cycle permission mode (ask → auto → full)', source: 'extension' },
    { name: '/plan', description: 'Toggle plan mode', source: 'extension' },
    { name: '/memory', description: 'Browse the memory store', source: 'extension' },
    { name: '/skills:editor', description: 'Skill: editor conventions', source: 'skill' },
    { name: '/model', description: 'Pick the model', source: 'builtin' },
  ];
  return mock;
}

function richFs() {
  const tree = genFsTree(CWD);
  const src = tree[CWD]!.kind === 'dir' ? tree[CWD]!.children.src! : undefined;
  if (src?.kind === 'dir') {
    src.children = {
      ...src.children,
      'tally.ts': { kind: 'file', text: 'export const tally = (xs: number[]) => xs.reduce((a, b) => a + b, 0);\n', mtime: 0 },
      'find.ts': { kind: 'file', text: 'export function findIndex(list: string[], q: string) {\n  return list.findIndex((x) => x.includes(q));\n}\n', mtime: 0 },
      'settings.ts': { kind: 'file', text: 'export const settings = {};\n', mtime: 0 },
    };
  }
  const root = tree[CWD]!;
  if (root.kind === 'dir') {
    root.children = {
      ...root.children,
      'tally.test.ts': { kind: 'file', text: 'import { tally } from "./src/tally";\n', mtime: 0 },
      docs: { kind: 'dir', children: { 'find.md': { kind: 'file', text: '# Find\n\nHow the bar wraps.\n', mtime: 0 } } },
    };
  }
  return tree;
}

function hosts(): MockScenario['hosts'] {
  return [
    {
      id: LOCAL,
      name: 'This computer',
      transport: 'local',
      conn: { state: 'connected' },
      cards: genCards(12, { seed: 5, root: '/home/you/projects', changes: true, unread: 2 }),
      sessions: { s001: commandSession() },
      fs: richFs(),
    },
    { id: 'pc-coffee', name: 'coffee', conn: { state: 'connected' }, cards: genCards(3, { seed: 9, root: '/home/me/work' }).map((c) => ({ ...c, session_id: `c${c.session_id}` })) },
  ];
}

function seed(session = 's001'): void {
  localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 480, bottomOpen: false, bottomH: 220 }));
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: LOCAL, sessionId: session }));
  // Z-78: a seeded search history for the chips.
  localStorage.setItem('unipi.cmdk.history', JSON.stringify({ [`${LOCAL}|${CWD}`]: [{ q: 'tally', scope: 'all' }, { q: '#flaky', scope: 'sessions' }, { q: '@tally', scope: 'files' }] }));
}

export const cmdkEmpty: MockScenario = { name: 'cmdk-empty', hosts: hosts(), setup: () => seed() };
export const cmdkFiles: MockScenario = { name: 'cmdk-files', hosts: hosts(), setup: () => seed() };
export const findBar: MockScenario = { name: 'find-bar', hosts: hosts(), setup: () => seed() };
export const shortcutsList: MockScenario = { name: 'shortcuts-list', hosts: hosts(), setup: () => seed() };
