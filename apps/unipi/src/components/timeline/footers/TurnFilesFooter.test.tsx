// @vitest-environment jsdom
/**
 * Turn file summary footer (WP-A27, app spec §3.13 / D-1) over a real
 * `createSession` + MockHost that answers the extension's services
 * (`_oino/service_call{target:"unipi.file-summary"}`) from a mutable fixture:
 * the collapsed/expanded footer, the lazy inline diff, undo per file / all,
 * the conflict prompt, the 500 ms refresh of a running turn and the cache the
 * A7 edit menu reads. Screenshots live in the capture scenarios
 * (`turn-files*`).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { dropSessionExpansion } from '../../../lib/timeline/expansion';
import { createMockHost, type MockScenario, type MockWorld } from '../../../lib/host/mock';
import { resetSessions, sessions, type SessionHandle } from '../../../lib/session/store';
import { setHostClientFactory } from '../../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { setPlatformOverride } from '../../../lib/platform';
import { actionNotice, clearActionNotice } from '../../../lib/timeline/actions';
import { toasts } from '../../../lib/toasts';
import { installUiSinks, resetUiRegistries, sessionUi } from '../../../ui/registries';
import { installUiEffects, resetUiEffects } from '../../../ui/slots/effects';
import { panes, resetPanes } from '../../../lib/panes';
import { TURN_FILES_REFRESH_MS, canUndoFiles, resetFileSummaries, type TurnFileRecord } from '../../../lib/fileSummary';
import { TurnFilesFooter } from './TurnFilesFooter';
import roundtrip from '../../../lib/session/fixtures/roundtrip.json';
import type { SessionSnapshot } from '../../../lib/host/client';
import type { UiEntry } from '../../../ui/types';

const HOST = 'h1';
const SID = 's1';
const TURN = 'u1';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

const file = (path: string, over: Partial<TurnFileRecord> = {}): TurnFileRecord => ({
  path,
  existed: true,
  beforeSha: 'aa',
  afterSha: 'bb',
  add: 4,
  del: 1,
  ...over,
});

type Fixture = {
  files: TurnFileRecord[];
  undonePaths: string[];
  running: boolean;
  diff: { [path: string]: string | null };
  /** Undo answers: `conflicts` names the paths a non-forced undo must not touch. */
  conflicts: string[];
  /** Force is answered as a clean undo of every requested path. */
  log: Array<{ name: string; payload: Record<string, unknown> }>;
};

let fx: Fixture;
let world: MockWorld;
let session: SessionHandle | undefined;

/** The scenario session's `_oino/service_call` handler (the extension's four services). */
function serviceCall(cmd: { name?: string; payload?: Record<string, unknown> }): unknown {
  const name = String(cmd.name ?? '');
  const payload = (cmd.payload ?? {}) as Record<string, unknown>;
  fx.log.push({ name, payload });
  switch (name) {
    case 'turn':
      return { turnEntryId: payload.turnEntryId, files: fx.files, undone: fx.undonePaths.length > 0, undonePaths: fx.undonePaths, running: fx.running };
    case 'diff': {
      const path = String(payload.path ?? '');
      const text = fx.diff[path];
      return text
        ? { turnEntryId: payload.turnEntryId, path, diff: text, truncated: false }
        : { turnEntryId: payload.turnEntryId, path, diff: null, reason: fx.files.find((f) => f.path === path)?.existed === false ? 'no_snapshot' : 'not_found' };
    }
    case 'undo': {
      const paths = (payload.paths as string[] | undefined) ?? fx.files.map((f) => f.path);
      const force = payload.force === true;
      const conflicts = paths.filter((p) => fx.conflicts.includes(p));
      if (conflicts.length && !force) return { restored: [], deleted: [], conflicts, failed: [], found: true };
      const restored = paths.filter((p) => fx.files.find((f) => f.path === p)?.existed !== false);
      const deleted = paths.filter((p) => fx.files.find((f) => f.path === p)?.existed === false);
      fx.undonePaths = [...new Set([...fx.undonePaths, ...restored, ...deleted])];
      return { restored, deleted, conflicts: [], failed: [], found: true };
    }
    case 'undoFrom':
      return { turns: [TURN], restored: [], deleted: [], conflicts: fx.conflicts, failed: [] };
    default:
      return {};
  }
}

const entry: UiEntry = {
  id: 'unipi.file-summary/turn-files',
  extensionId: 'unipi.file-summary',
  scope: 'session',
  slot: 'turnFooter',
  order: 0,
  seq: 1,
  rev: 1,
  view: { kind: 'reserved', component: 'turnFiles' },
};

beforeEach(() => {
  localStorage.clear();
  resetSessions();
  resetUiRegistries();
  installUiSinks();
  installUiEffects();
  resetPanes();
  resetFileSummaries();
  clearActionNotice();
  resetLayout();
  setDesktopOverride(true);
  setPlatformOverride('tauri-desktop');
  fx = {
    files: [file('src/math.ts', { add: 12, del: 4 }), file('docs/README.md', { existed: false, beforeSha: undefined, add: 3, del: 0 }), file('assets/big.bin', { add: 0, del: 0, skipped: 'too_large' })],
    undonePaths: [],
    running: false,
    diff: { 'src/math.ts': '--- a/src/math.ts\n+++ b/src/math.ts\n@@ -1,2 +1,2 @@\n-return a - b;\n+return a + b;\n' },
    conflicts: [],
    log: [],
  };
  const scenario: MockScenario = {
    name: 'turn-files',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        sessions: {
          [SID]: {
            snapshot: { ...(structuredClone(rt.fresh) as SessionSnapshot), seq: 0, entries: [] },
            events: [],
            cmd: { '_oino/service_call': (cmd: unknown) => serviceCall(cmd as { name?: string; payload?: Record<string, unknown> }) },
          },
        },
      },
    ],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
});

afterEach(() => {
  dropSessionExpansion(SID);
  session = undefined;
  cleanup();
  resetSessions();
  resetUiEffects();
  resetUiRegistries();
  resetPanes();
  setHostClientFactory(undefined);
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  resetFileSummaries();
  clearActionNotice();
});

const calls = (name: string) => fx.log.filter((c) => c.name === name);

async function open(): Promise<SessionHandle> {
  const s = sessions().ensure(HOST, SID);
  session = s;
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return s;
}

const footer = () => render(() => <TurnFilesFooter entry={entry} target={{ scope: 'session', hostId: HOST, sessionId: SID, turnEntryId: TURN }} />);

describe('turn file summary footer (A27)', () => {
  it('collapsed: "N files +add −del"; expanding lists path, counts and status (skipped included)', async () => {
    await open();
    footer();
    await waitFor(() => expect(screen.getByTestId('turn-files')).toBeTruthy());
    expect(screen.getByTestId('turn-files-stat').textContent).toBe('+15 −04');
    expect(screen.getByTestId('turn-files-toggle').textContent).toContain('3 files');
    // collapsed = one row only: the file list is not mounted
    expect(screen.queryByTestId('turn-file-src/math.ts')).toBeNull();
    fireEvent.click(screen.getByTestId('turn-files-toggle'));
    await waitFor(() => expect(screen.getByTestId('turn-file-src/math.ts')).toBeTruthy());
    const row = screen.getByTestId('turn-file-src/math.ts').closest('.tl-files-row') as HTMLElement;
    expect(row.dataset.status).toBe('modified');
    expect(row.textContent).toContain('math.ts');
    expect(row.textContent).toContain('+12 −4');
    expect((screen.getByTestId('turn-file-docs/README.md').closest('.tl-files-row') as HTMLElement).dataset.status).toBe('new');
    const skipped = screen.getByTestId('turn-file-assets/big.bin').closest('.tl-files-row') as HTMLElement;
    expect(skipped.dataset.status).toBe('skipped');
    // a skipped file has no snapshot: no diff and no undo
    expect(screen.queryByTestId('turn-file-undo-assets/big.bin')).toBeNull();
    expect(screen.queryByTestId('turn-file-diff-assets/big.bin')).toBeNull();
    // and the collapsed header only asked once
    expect(calls('turn')).toHaveLength(1);
  });

  it('a settled turn is asked once and re-read on invalidateTurnFooter, never more', async () => {
    const s = await open();
    footer();
    await waitFor(() => expect(calls('turn')).toHaveLength(1));
    await new Promise((r) => setTimeout(r, TURN_FILES_REFRESH_MS + 120));
    expect(calls('turn')).toHaveLength(1);
    // the extension's `invalidateTurnFooter` (§3.13) arrives as a ui invalidation
    const rev = sessionUi(HOST, SID).state.rev + 1;
    world.push(HOST, SID, [{ session: SID, seq: s.state.seq + 1, event: { type: 'oino_ui_patch', scope: 'session', rev, ops: [{ op: 'invalidate', id: entry.id }] } }]);
    await waitFor(() => expect(calls('turn')).toHaveLength(2));
  });

  it('a running turn is re-read at most every 500 ms', async () => {
    fx.running = true;
    await open();
    footer();
    await waitFor(() => expect(calls('turn').length).toBeGreaterThan(0));
    const start = Date.now();
    await new Promise((r) => setTimeout(r, TURN_FILES_REFRESH_MS * 3 + 150));
    const elapsed = Date.now() - start;
    const n = calls('turn').length;
    // initial + one per tick; never more than one per interval
    expect(n).toBeLessThanOrEqual(Math.ceil(elapsed / TURN_FILES_REFRESH_MS) + 2);
    expect(n).toBeGreaterThanOrEqual(3);
  });

  it('a clicked row fetches its diff lazily, once, and closes again', async () => {
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(await screen.findByTestId('turn-file-src/math.ts'));
    await waitFor(() => expect(screen.getByTestId('turn-file-body-src/math.ts')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('turn-file-body-src/math.ts').querySelector('.diffview')).toBeTruthy());
    expect(screen.getByTestId('turn-file-body-src/math.ts').textContent).toContain('return a + b;');
    expect(calls('diff')).toHaveLength(1);
    expect(calls('diff')[0]!.payload).toMatchObject({ turnEntryId: TURN, path: 'src/math.ts' });
    // close, reopen: the diff is cached for this turn
    fireEvent.click(screen.getByTestId('turn-file-src/math.ts'));
    await waitFor(() => expect(screen.queryByTestId('turn-file-body-src/math.ts')).toBeNull());
    fireEvent.click(screen.getByTestId('turn-file-src/math.ts'));
    await waitFor(() => expect(screen.getByTestId('turn-file-body-src/math.ts')).toBeTruthy());
    expect(calls('diff')).toHaveLength(1);
  });

  it('a file the turn created says there is no snapshot instead of showing an empty diff', async () => {
    fx.diff = {};
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(await screen.findByTestId('turn-file-docs/README.md'));
    await waitFor(() => expect(screen.getByTestId('turn-file-body-docs/README.md')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('turn-file-body-docs/README.md').textContent).toContain('No snapshot of this file'));
  });

  it('Undo on a row sends `undo{paths:[path]}`, marks the row undone and re-reads the summary', async () => {
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(await screen.findByTestId('turn-file-undo-src/math.ts'));
    await waitFor(() => expect(calls('undo')).toHaveLength(1));
    expect(calls('undo')[0]!.payload).toEqual({ turnEntryId: TURN, paths: ['src/math.ts'] });
    await waitFor(() => expect((screen.getByTestId('turn-file-src/math.ts').closest('.tl-files-row') as HTMLElement).dataset.status).toBe('undone'));
    expect(screen.queryByTestId('turn-file-undo-src/math.ts')).toBeNull();
    expect(screen.getByTestId('turn-files-toggle').textContent).toContain('undone');
    expect(toasts().list().map((t) => t.message).join(' ')).toContain('1 file undone');
    await waitFor(() => expect(calls('turn').length).toBeGreaterThan(1));
  });

  it('Undo all asks first; Cancel sends nothing, confirming sends `undo{turnEntryId}`', async () => {
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(await screen.findByTestId('turn-files-undo-all'));
    expect(screen.getByTestId('turn-files-undo-confirm')).toBeTruthy();
    fireEvent.click(screen.getByTestId('turn-files-undo-cancel'));
    expect(screen.queryByTestId('turn-files-undo-confirm')).toBeNull();
    expect(calls('undo')).toHaveLength(0);
    fireEvent.click(screen.getByTestId('turn-files-undo-all'));
    fireEvent.click(screen.getByTestId('turn-files-undo-confirm'));
    await waitFor(() => expect(calls('undo')).toHaveLength(1));
    expect(calls('undo')[0]!.payload).toEqual({ turnEntryId: TURN });
    await waitFor(() => expect(screen.getByTestId('turn-files-toggle').textContent).toContain('undone'));
  });

  it('a conflict asks "Changed since this turn — undo anyway?" and only then forces', async () => {
    fx.conflicts = ['src/math.ts'];
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(await screen.findByTestId('turn-file-undo-src/math.ts'));
    const prompt = await screen.findByTestId('turn-files-conflict');
    expect(prompt.textContent).toContain('Changed since this turn — undo anyway?');
    expect(prompt.textContent).toContain('src/math.ts');
    // the non-forced attempt did not restore anything
    expect(calls('undo')[0]!.payload).toEqual({ turnEntryId: TURN, paths: ['src/math.ts'] });
    fireEvent.click(screen.getByTestId('turn-files-conflict-cancel'));
    expect(screen.queryByTestId('turn-files-conflict')).toBeNull();
    expect(calls('undo')).toHaveLength(1);
    // …and "Undo anyway" sends the same paths with force
    fx.conflicts = ['src/math.ts'];
    fireEvent.click(screen.getByTestId('turn-file-undo-src/math.ts'));
    fireEvent.click(await screen.findByTestId('turn-files-conflict-confirm'));
    await waitFor(() => expect(calls('undo')).toHaveLength(3));
    expect(calls('undo')[2]!.payload).toEqual({ turnEntryId: TURN, paths: ['src/math.ts'], force: true });
    await waitFor(() => expect((screen.getByTestId('turn-file-src/math.ts').closest('.tl-files-row') as HTMLElement).dataset.status).toBe('undone'));
  });

  it('"View diff" opens the diff tab addressed by turn + path (no tool call)', async () => {
    await open();
    footer();
    fireEvent.click(await screen.findByTestId('turn-files-toggle'));
    fireEvent.click(screen.getByTestId('turn-file-diff-src/math.ts'));
    const scope = { hostId: HOST, projectKey: '~' };
    const tabs = panes().visible(scope, 'side', { sessionId: SID });
    const diff = tabs.find((t) => t.kind === 'diff');
    expect(diff).toBeTruthy();
    expect(diff!.params).toMatchObject({ turnEntryId: TURN, path: 'src/math.ts', hostId: HOST, sessionId: SID });
  });

  it('publishes the summary the A7 edit menu reads (also undo file changes)', async () => {
    const s = await open();
    footer();
    await waitFor(() => expect(calls('turn')).toHaveLength(1));
    await waitFor(() => expect(canUndoFiles(s.key, TURN)).toBe(true));
    fx.undonePaths = fx.files.map((f) => f.path);
    const { refreshTurnSummary } = await import('../../../lib/fileSummary');
    await refreshTurnSummary(s, TURN);
    expect(canUndoFiles(s.key, TURN)).toBe(false); // nothing left to undo
  });

  it('an empty summary renders nothing (a turn that touched no file)', async () => {
    fx.files = [];
    await open();
    footer();
    await waitFor(() => expect(calls('turn')).toHaveLength(1));
    expect(screen.queryByTestId('turn-files')).toBeNull();
  });
});
