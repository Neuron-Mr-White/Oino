/**
 * Turn file summary footer scenarios (WP-A27, app spec §3.13 / D-1):
 *
 *   turn-files           the collapsed "3 files +15 −04" under two settled
 *                        turns (one with fewer files), plus a `skipped` row
 *                        visible once expanded
 *   turn-files-undo      the same turn expanded with one path already undone
 *                        ("· undone" in the header, the row struck through)
 *   turn-files-conflict  expand + Undo on a file the workspace changed since:
 *                        the "Changed since this turn — undo anyway?" prompt
 *
 * One session with two hand-built turns (so the footer's `turnEntryId` is
 * `u1`/`u2`); `_oino/service_call` answers the extension's `turn` / `diff` /
 * `undo` services from the fixture, the way `unipi.file-summary` does.
 */
import type { MockScenario, MockSession } from '../host';
import { BASE_TIME, genCards, genFsTree, genSession } from '../gen';
import type { SessionEntry } from '../../../lib/session/types';

const HOST = 'pc-coffee';
const S = 's-files';
const CWD = '/home/you/projects/unipi';

const text = (t: string) => [{ type: 'text', text: t }];
const call = (id: string, name: string, args: Record<string, unknown>) => ({ type: 'toolCall', id, name, arguments: args });

type FileRecord = { path: string; existed: boolean; beforeSha?: string; afterSha?: string; add: number; del: number; skipped?: string };

const MATH: FileRecord = { path: 'src/math.ts', existed: true, beforeSha: 'a1', afterSha: 'b2', add: 12, del: 4 };
const README: FileRecord = { path: 'docs/README.md', existed: false, afterSha: 'c3', add: 3, del: 0 };
const BINARY: FileRecord = { path: 'assets/logo.bin', existed: true, add: 0, del: 0, skipped: 'too_large' };
const TEST: FileRecord = { path: 'src/math.test.ts', existed: true, beforeSha: 'd4', afterSha: 'e5', add: 8, del: 2 };

const DIFF = [
  '--- a/src/math.ts',
  '+++ b/src/math.ts',
  '@@ -1,4 +1,4 @@',
  ' export function add(a: number, b: number) {',
  '-  return a - b;',
  '+  return a + b;',
  ' }',
  '@@ -12,3 +12,3 @@',
  '-export const VERSION = "1.0.0";',
  '+export const VERSION = "1.0.1";',
  '',
].join('\n');

/** The two turns: an edit + its result, a question and an answer. */
function entries(): SessionEntry[] {
  let t = BASE_TIME;
  const out: SessionEntry[] = [];
  const add = (entry: Record<string, unknown>) => {
    t += 3000;
    out.push({ ...entry, timestamp: new Date(t).toISOString() } as SessionEntry);
  };
  add({ type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: text('Fix add() — it subtracts') } });
  add({
    type: 'message',
    id: 'a1',
    parentId: 'u1',
    message: {
      role: 'assistant',
      content: [call('tc-edit', 'edit', { path: 'src/math.ts', edits: [{ oldText: 'return a - b;', newText: 'return a + b;' }] })],
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      stopReason: 'toolUse',
      usage: { input: 1800, output: 60, totalTokens: 1860, cost: { total: 0.005 } },
    },
  });
  add({ type: 'message', id: 'r1', parentId: 'a1', message: { role: 'toolResult', toolCallId: 'tc-edit', toolName: 'edit', content: text('Successfully replaced 1 block in src/math.ts.') } });
  add({ type: 'message', id: 'a2', parentId: 'r1', message: { role: 'assistant', content: text('`add()` adds now, and the version went to 1.0.1.'), stopReason: 'stop' } });
  add({ type: 'message', id: 'u2', parentId: 'a2', message: { role: 'user', content: text('Add a test for the version bump') } });
  add({
    type: 'message',
    id: 'a3',
    parentId: 'u2',
    message: {
      role: 'assistant',
      content: [call('tc-edit2', 'edit', { path: 'src/math.test.ts', edits: [{ oldText: 'it("adds")', newText: 'it("adds and bumps")' }] })],
      provider: 'anthropic',
      model: 'claude-sonnet-4-5',
      stopReason: 'toolUse',
    },
  });
  add({ type: 'message', id: 'r2', parentId: 'a3', message: { role: 'toolResult', toolCallId: 'tc-edit2', toolName: 'edit', content: text('Successfully replaced 1 block in src/math.test.ts.') } });
  add({ type: 'message', id: 'a4', parentId: 'r2', message: { role: 'assistant', content: text('The test now covers the bump.'), stopReason: 'stop' } });
  return out;
}

/** `unipi.file-summary/turn-files` as the manifest declares it (§8, §3.13). */
const FOOTER_ENTRY = {
  id: 'unipi.file-summary/turn-files',
  extensionId: 'unipi.file-summary',
  scope: 'session',
  slot: 'turnFooter',
  order: 0,
  seq: 1,
  rev: 1,
  view: { kind: 'reserved', component: 'turnFiles' },
};

type Fixture = {
  /** Turn entry id → the files that turn touched (the `turn` service). */
  files: Record<string, FileRecord[]>;
  /** Paths an `undo` already put back (the "undone" state). */
  undone?: string[];
  /** Paths a non-forced `undo` must refuse (changed since the turn). */
  conflicts?: string[];
};

function services(fixture: Fixture) {
  const undone = new Set(fixture.undone ?? []);
  return (cmd: { name?: string; payload?: Record<string, unknown> }) => {
    const payload = (cmd.payload ?? {}) as Record<string, unknown>;
    const turnEntryId = String(payload.turnEntryId ?? '');
    const files = fixture.files[turnEntryId] ?? [];
    switch (cmd.name) {
      case 'turn':
        return {
          turnEntryId,
          files,
          undone: files.some((file) => undone.has(file.path)),
          undonePaths: files.map((file) => file.path).filter((path) => undone.has(path)),
          running: false,
        };
      case 'diff': {
        const path = String(payload.path ?? '');
        return path === 'src/math.ts'
          ? { turnEntryId, path, diff: DIFF, truncated: false }
          : { turnEntryId, path, diff: null, reason: files.find((file) => file.path === path)?.existed === false ? 'no_snapshot' : 'not_found' };
      }
      case 'undo': {
        const paths = (payload.paths as string[] | undefined) ?? files.map((file) => file.path);
        const force = payload.force === true;
        const conflicts = paths.filter((path) => (fixture.conflicts ?? []).includes(path));
        if (conflicts.length && !force) return { restored: [], deleted: [], conflicts, failed: [], found: true };
        for (const path of paths) undone.add(path);
        return { restored: paths.filter((path) => files.find((file) => file.path === path)?.existed !== false), deleted: [], conflicts: [], failed: [], found: true };
      }
      default:
        return {};
    }
  };
}

function scenario(name: string, fixture: Fixture): MockScenario {
  const base = genSession(S, { seed: 11, turns: 2, toolsPerTurn: 1, cwd: CWD });
  const session: MockSession = {
    ...base,
    snapshot: {
      ...base.snapshot,
      entries: entries(),
      leaf_id: 'a4',
      ui: { scope: 'session', sessionId: S, rev: 1, dialogs: [], meta: {}, entries: [FOOTER_ENTRY] } as never,
    },
    events: [],
    cmd: { ...(base.cmd ?? {}), '_oino/service_call': (cmd: unknown) => services(fixture)(cmd as { name?: string; payload?: Record<string, unknown> }) },
  };
  return {
    name,
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [{ session_id: S, title: 'Fix add() — it subtracts', cwd: CWD, last_activity: BASE_TIME }, ...genCards(4, { seed: 5, root: CWD })] as never[],
        fs: genFsTree(CWD),
        // The extension is installed and on (the §8 owner gate).
        settings: { extensions: ['unipi.file-summary', 'unipi.kanban', 'unipi.cron', 'unipi.info-screen', 'unipi.long-horizon'].map((id) => ({ id, enabled: true })) },
        sessions: { [S]: session },
      },
    ],
    speed: 0,
    setup: () => {
      localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: S }));
      localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: false, sideW: 460, bottomOpen: false, bottomH: 220 }));
    },
  };
}

const FILES: Record<string, FileRecord[]> = { u1: [MATH, README, BINARY], u2: [TEST] };

/** Collapsed: one line per turn, `3 files +15 −04` and `1 file +08 −02`. */
export const turnFiles = scenario('turn-files', { files: FILES });

/** Expanded with `src/math.ts` undone (the header's "· undone", the row struck through). */
export const turnFilesUndo = scenario('turn-files-undo', { files: FILES, undone: ['src/math.ts'] });

/** Undo on a file the workspace changed since → the force prompt. */
export const turnFilesConflict = scenario('turn-files-conflict', { files: FILES, conflicts: ['src/math.ts', 'docs/README.md'] });
