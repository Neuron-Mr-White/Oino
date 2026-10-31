// @vitest-environment jsdom
/**
 * The `diff` tab (WP-A10, Z-40): it resolves a tool call from the session by
 * its id (so a persisted tab survives a reload), pages across the call's
 * edits, renders unified **and** split (desktop), and "Open in editor" lands
 * on the edit's first changed line.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { createMockHost, type MockScenario } from '../../../dev/mock/host';
import { setHostClientFactory } from '../../../lib/host/factory';
import { createSession, resetSessions, sessions } from '../../../lib/session/store';
import { panes, resetPanes } from '../../../lib/panes';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { clearEditorTarget, getEditorTarget } from '../../../lib/editorTarget';
import { allAdditions, editDiff } from '../../../lib/markdown';
import { DiffTab, editsOfArgs, findToolCall, renderSplitHtml, splitRows } from './DiffTab';
import type { PaneTabProps } from '../../../lib/panes/kinds';
import type { SessionEntry } from '../../../lib/session/types';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-one';
const CWD = '/home/you/unipi';
const S = 's-diff';

const text = (t: string) => [{ type: 'text', text: t }];

const ENTRIES: SessionEntry[] = [
  { type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: text('Fix add') } },
  {
    type: 'message',
    id: 'a1',
    parentId: 'u1',
    message: {
      role: 'assistant',
      content: [
        {
          type: 'toolCall',
          id: 'tc-edit',
          name: 'edit',
          arguments: {
            path: 'src/math.ts',
            edits: [
              { oldText: 'export function add(a, b) {\n  return a - b;\n}', newText: 'export function add(a, b) {\n  return a + b;\n}' },
              { oldText: 'const x = 1;', newText: 'const x = 2;' },
            ],
          },
        },
      ],
      stopReason: 'toolUse',
    },
  },
  { type: 'message', id: 'r1', parentId: 'a1', message: { role: 'toolResult', toolCallId: 'tc-edit', toolName: 'edit', content: text('Replaced 2 blocks.') } },
  {
    type: 'message',
    id: 'a2',
    parentId: 'r1',
    message: { role: 'assistant', content: [{ type: 'toolCall', id: 'tc-write', name: 'write', arguments: { path: 'docs/notes.md', content: '# Notes\n\n- one\n' } }], stopReason: 'toolUse' },
  },
];

const scenario = (): MockScenario => ({
  name: 'diff-test',
  hosts: [
    {
      id: HOST,
      name: 'coffee',
      conn: { state: 'connected' },
      cards: [{ session_id: S, title: 'Diff', cwd: CWD, last_activity: 1 }] as never[],
      sessions: {
        [S]: {
          snapshot: { session_id: S, reset: true, seq: 0, state: { sessionId: S, isStreaming: false }, entries: ENTRIES, leaf_id: 'a2', commands: [], ui: { scope: 'session', sessionId: S, rev: 0, entries: [], dialogs: [] }, topics: [] },
        },
      },
    },
  ],
});

let world: ReturnType<typeof createMockHost>;

beforeEach(() => {
  world = createMockHost(scenario());
  setHostClientFactory(world.client);
  localStorage.clear();
  resetPanes();
  resetSessions();
  resetLayout();
  setDesktopOverride(true);
  clearEditorTarget();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  setDesktopOverride(undefined);
  vi.restoreAllMocks();
});

const props = (params: Record<string, unknown>): PaneTabProps => ({
  tab: { id: 't-diff', kind: 'diff', params: { hostId: HOST, sessionId: S, path: 'src/math.ts', ...params }, sessionId: S },
  area: 'side',
  hostId: HOST,
  projectKey: CWD,
  sessionId: S,
  close: () => {},
  setTitle: () => {},
});

function withRouter(body: () => JSX.Element) {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => body()} />
    </HashRouter>
  ));
}

async function live() {
  const handle = createSession(HOST, S, { client: world.client(HOST) });
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 10));
  return handle;
}

describe('pure diff helpers', () => {
  it('finds a tool call by id, newest first', () => {
    const site = findToolCall(ENTRIES, 'tc-edit');
    expect(site?.entryId).toBe('a1');
    expect(site?.part).toBe(0);
    expect((site?.args.edits as unknown[]).length).toBe(2);
    expect(findToolCall(ENTRIES, 'nope')).toBeUndefined();
  });

  it('a write call is one all-additions edit', () => {
    const edits = editsOfArgs(findToolCall(ENTRIES, 'tc-write')!.args);
    expect(edits).toHaveLength(1);
    expect(edits[0]!.hunk.lines.every((l) => l.kind === 'add')).toBe(true);
    expect(edits[0]!.line).toBe(1);
  });

  it('an edit pair carries the first changed line (the editor target)', () => {
    const edits = editsOfArgs(findToolCall(ENTRIES, 'tc-edit')!.args);
    expect(edits).toHaveLength(2);
    const hunk = editDiff('a\nb\n', 'a\nc\n');
    expect(edits[0]!.line).toBe(hunk.lines.find((l) => l.kind === 'add')!.newNo);
  });

  it('split rows pair deletions with the additions that follow', () => {
    const rows = splitRows([editDiff('a\nb\n', 'a\nbb\ncc\n')]);
    const pairs = rows.filter((r) => r.kind === 'pair');
    // del run and add run side by side, the shorter side padded with a gap
    expect(pairs.some((p) => p.old?.kind === 'del' && p.new?.kind === 'add')).toBe(true);
    expect(pairs.some((p) => p.old === undefined && p.new?.kind === 'add')).toBe(true);
  });

  it('the split HTML marks gaps and both cells', () => {
    const html = renderSplitHtml(allAdditions('only\n').files[0]!.hunks);
    expect(html).toContain('diffview-split');
    expect(html).toContain('diff-gap');
    expect(html).toContain('only');
  });
});

describe('diff tab', () => {
  it("renders a turn's file-summary diff by turn + path (A27's \"View diff\")", async () => {
    const asked: Array<Record<string, unknown>> = [];
    const sc = scenario();
    const base = sc.hosts[0]!.sessions![S]!;
    sc.hosts[0]!.sessions![S] = {
      ...base,
      cmd: {
        '_oino/service_call': (cmd: unknown) => {
          const c = cmd as { name?: string; payload?: Record<string, unknown> };
          asked.push({ name: c.name, ...(c.payload ?? {}) });
          return c.name === 'diff'
            ? {
                turnEntryId: c.payload?.turnEntryId,
                path: c.payload?.path,
                diff: '--- a/src/math.ts\n+++ b/src/math.ts\n@@ -1,2 +1,2 @@\n-return a - b;\n+return a + b;\n@@ -9,2 +9,2 @@\n-const x = 1;\n+const x = 2;\n',
                truncated: false,
              }
            : {};
        },
      },
    };
    world = createMockHost(sc);
    setHostClientFactory(world.client);
    const handle = sessions().ensure(HOST, S);
    await waitFor(() => expect(handle.state.conn).toBe('live'));
    setDesktopOverride(true);
    withRouter(() => <DiffTab {...props({ turnEntryId: 'u1' })} />);
    await waitFor(() => expect(document.querySelector('.diffview')).toBeTruthy());
    expect(document.querySelector('.diffview')!.textContent).toContain('return a + b;');
    expect(asked).toEqual([{ name: 'diff', turnEntryId: 'u1', path: 'src/math.ts' }]);
    // one page per hunk of the turn's diff
    expect(document.body.textContent).toContain('edit 1 of 2');
  });

  it('resolves the call from the session (a persisted tab has only the id)', async () => {
    await live();
    withRouter(() => <DiffTab {...props({ toolId: 'tc-edit' })} />);
    await waitFor(() => expect(screen.getByTestId('diff-tab')).toBeTruthy());
    await waitFor(() => expect(document.querySelector('.diffview')).toBeTruthy());
    expect(document.querySelector('.diffview')!.textContent).toContain('return a + b;');
    // Two edits → the pager, with the first active.
    expect(screen.getByTestId('diff-stat').textContent).toMatch(/\+\d+ −\d+/);
    expect(document.body.textContent).toContain('edit 1 of 2');
  });

  it('pages across the edits and toggles the split view (desktop)', async () => {
    await live();
    withRouter(() => <DiffTab {...props({ toolId: 'tc-edit' })} />);
    await waitFor(() => expect(document.querySelector('.diffview')).toBeTruthy());
    fireEvent.click(screen.getByTestId('diff-next'));
    await waitFor(() => expect(document.querySelector('.diffview')!.textContent).toContain('const x = 2;'));
    fireEvent.click(screen.getByTestId('diff-prev'));
    await waitFor(() => expect(document.querySelector('.diffview')!.textContent).toContain('return a + b;'));
    fireEvent.click(screen.getByTestId('diff-split'));
    await waitFor(() => expect(document.querySelector('.diffview-split')).toBeTruthy());
    expect(document.querySelectorAll('.diffview-split .diff-split').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByTestId('diff-unified'));
    await waitFor(() => expect(document.querySelector('.diffview-split')).toBeNull());
  });

  it('"Open in editor" targets the file at the edit line and opens the Files tab', async () => {
    await live();
    withRouter(() => <DiffTab {...props({ toolId: 'tc-edit' })} />);
    await waitFor(() => expect(screen.getByTestId('diff-open-editor')).toBeTruthy());
    fireEvent.click(screen.getByTestId('diff-open-editor'));
    const target = getEditorTarget();
    expect(target).toMatchObject({ hostId: HOST, path: `${CWD}/src/math.ts` });
    expect(target!.line).toBeGreaterThan(0);
    await waitFor(() => expect(panes().visible({ hostId: HOST, projectKey: CWD }, 'side', { sessionId: S }).some((t) => t.kind === 'files')).toBe(true));
  });

  it('a call that is not in the session shows the empty state (a stale tab)', async () => {
    await live();
    withRouter(() => <DiffTab {...props({ toolId: 'gone' })} />);
    await waitFor(() => expect(screen.getByText('This edit has no diff to show.')).toBeTruthy());
  });
});
