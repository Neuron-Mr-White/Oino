// @vitest-environment jsdom
/**
 * The Git section of the status pane (WP-A15): sources (Z-96), write actions
 * (Z-97), refresh on `git_changed` only (Z-98) — over a fake `HostClient`
 * (the same seam the capture harness uses).
 */
import { render, screen, waitFor, cleanup, fireEvent, within } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { createSignal } from 'solid-js';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import type { AppError } from '@unipi/protocol';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import type { MockScenario } from '../../dev/mock/host';
import type { Closable, HostClient, HubEvent, HostConn, SessionStream, SessionSink } from '../../lib/host/client';
import { closeGit, openGit } from '../../lib/git';
import { resetHubs } from '../../lib/session/hub';
import { resetSessions } from '../../lib/session/store';
import { setPlatformOverride } from '../../lib/platform';
import { resetLayout } from '../../lib/layout';
import { getEditorTarget, clearEditorTarget } from '../../lib/editorTarget';
import { GitSection } from './GitSection';
import { StatusPaneLayer } from './StatusPane';

// jsdom's ResizeObserver stub (test-setup) never reports sizes, so the real
// virtualizer renders nothing here. This windowed stand-in renders every row
// (the 2 000-file behaviour is the capture + bench's job, not jsdom's).
vi.mock('@tanstack/solid-virtual', () => {
  const all = (opts: { count?: number }) =>
    Array.from({ length: opts.count ?? 0 }, (_, i) => ({ index: i, key: i, start: i * 44, size: 44 }));
  return {
    createVirtualizer: (opts: { count?: number }) => ({
      get count() {
        return opts.count;
      },
      getVirtualItems: () => all(opts),
      getTotalSize: () => (opts.count ?? 0) * 44,
      measureElement: () => {},
      scrollToIndex: () => {},
    }),
  };
});

type Cmd = { action: string; [k: string]: unknown };
type Reply = { ok: true; value: unknown } | { ok: false; error: AppError };

const repo = { repo: true, root: '/home/u/proj', branch: 'main', head: 'abc1234', detached: false, upstream: 'origin/main', ahead: 2, behind: 1, remotes: ['origin'], stashes: 1 };
let files: Array<Record<string, unknown>>;
let calls: Array<{ op: string; args?: unknown }>;
let overrides: Partial<Record<string, (cmd: Cmd) => Reply>>;
let eventSinks: Array<(e: HubEvent) => void>;

const ENTRIES = [
  { type: 'message', id: 'e1', message: { role: 'user', content: 'fix it', timestamp: 1 } },
  {
    type: 'message',
    id: 'e2',
    message: {
      role: 'assistant',
      stopReason: 'endTurn',
      timestamp: 2,
      content: [
        { type: 'toolCall', id: 't1', name: 'write', arguments: { path: 'src/a.ts', content: 'x\ny\n' } },
        { type: 'toolCall', id: 't2', name: 'edit', arguments: { file_path: 'b.txt', oldText: 'x', newText: 'z' } },
        { type: 'text', text: 'done' },
      ],
    },
  },
];

function reply(cmd: Cmd): Reply {
  const o = overrides[cmd.action];
  if (o) return o(cmd);
  switch (cmd.action) {
    case 'repo': return { ok: true, value: repo };
    case 'status': return { ok: true, value: { ...repo, files, more: false } };
    case 'turn_files':
      return { ok: true, value: { ...repo, files: (cmd.files as string[]).map((path) => ({ path, area: 'unstaged', status: 'M', add: 2, del: 1, binary: false, diff: { text: `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-x\n+y\n`, truncated: false, binary: false } })) } };
    case 'branch_files':
      return { ok: true, value: { ...repo, base: 'origin/main', files: [{ path: 'src/a.ts', area: 'staged', status: 'M', add: 4, del: 2, binary: false, diff: { text: 'diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-a\n+b\n', truncated: false, binary: false } }] } };
    case 'diff': return { ok: true, value: { text: `diff --git a/${cmd.file} b/${cmd.file}\n--- a/${cmd.file}\n+++ b/${cmd.file}\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n`, truncated: false, binary: false } };
    case 'stage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path as string);
      files = files.map((f) => (paths.includes(f.path as string) ? { ...f, area: 'staged' } : f));
      return { ok: true, value: null };
    }
    case 'unstage': {
      const paths = (cmd.paths as string[] | undefined) ?? files.map((f) => f.path as string);
      files = files.map((f) => (paths.includes(f.path as string) && f.area === 'staged' ? { ...f, area: f.status === 'A' ? 'untracked' : 'unstaged' } : f));
      return { ok: true, value: null };
    }
    case 'commit':
      files = files.filter((f) => f.area !== 'staged');
      return { ok: true, value: { hash: 'def5678', subject: String(cmd.message).split('\n')[0] } };
    case 'discard':
      files = files.filter((f) => !(cmd.paths as string[]).includes(f.path as string));
      return { ok: true, value: null };
    case 'log': {
      const all = Array.from({ length: 60 }, (_, i) => ({ hash: `h${i}`.padEnd(40, '0'), short: `h${i}`, subject: `commit ${i}`, author: 'Ann', time: 1_700_000_000 - i * 3600, refs: i === 0 ? ['HEAD -> main', 'origin/main'] : [] }));
      const skip = Number(cmd.skip ?? 0);
      const limit = Number(cmd.limit ?? 50);
      return { ok: true, value: { commits: all.slice(skip, skip + limit), more: skip + limit < all.length } };
    }
    case 'show':
      return { ok: true, value: { commit: { hash: String(cmd.rev), short: String(cmd.rev).slice(0, 7), subject: 'commit 0', author: 'Ann', time: 1_700_000_000, refs: [] }, body: 'Longer body.', files: [{ path: 'a.txt', status: 'M', add: 1, del: 1 }], diff: 'diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\n', truncated: false } };
    case 'branches':
      return { ok: true, value: { branches: [{ name: 'main', current: true, upstream: 'origin/main', ahead: 2, behind: 1, time: 1_700_000_000, subject: 'x' }, { name: 'feat/x', time: 1_699_000_000, subject: 'y' }, { name: 'origin/main', remote: true }] } };
    case 'switch': return { ok: true, value: null };
    case 'push': return { ok: true, value: { output: 'To origin\n   abc..def  main -> main' } };
    case 'pull': return { ok: true, value: { output: 'Already up to date.' } };
    default: return { ok: true, value: { output: 'Done.' } };
  }
}

function fakeClient(hostId: string): HostClient {
  const [conn] = createSignal<HostConn>({ state: 'connected' });
  return {
    hostId,
    conn,
    retry: async () => {},
    async request<T>(op: string, args?: Record<string, unknown>): Promise<T> {
      calls.push({ op, args });
      if (op === 'git_watch') return { root: repo.root } as T;
      if (op !== 'git_op') return null as T;
      const r = reply(args!.cmd as Cmd);
      if (!r.ok) throw r.error;
      return r.value as unknown as T;
    },
    openSession(_req, sink: SessionSink): SessionStream {
      queueMicrotask(() => {
        sink.snapshot({ session_id: 's1', reset: true, seq: 2, entries: ENTRIES, leaf_id: 'e2' } as never);
      });
      return { cmd: (async () => ({})) as SessionStream['cmd'], close() {} };
    },
    openEvents(sink: (e: HubEvent) => void): Closable {
      eventSinks.push(sink);
      return { close() {} };
    },
    openTerm: () => ({ write() {}, resize() {}, close() {} }),
    openLsp: () => ({ send() {}, close() {} }),
  };
}

const gitOps = () => calls.filter((c) => c.op === 'git_op').map((c) => (c.args as { cmd: Cmd }).cmd);
const statusCalls = () => gitOps().filter((c) => c.action === 'status');

beforeEach(() => {
  files = [
    { path: 'src/a.ts', area: 'staged', status: 'M', add: 3, del: 1, binary: false },
    { path: 'b.txt', area: 'unstaged', status: 'M', add: 1, del: 1, binary: false },
    { path: 'new.md', area: 'untracked', status: '?', add: 12, binary: false },
  ];
  calls = [];
  overrides = {};
  eventSinks = [];
  resetLayout();
  resetHubs();
  resetSessions();
  setHostClientFactory(fakeClient as never);
});

afterEach(() => {
  closeGit();
  cleanup();
  setHostClientFactory(undefined);
  clearEditorTarget();
  setPlatformOverride(undefined);
  location.hash = '#/';
});

function mountPhone(path = '/home/u/proj/src', sessionId?: string) {
  setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: false });
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <GitSection hostId="h1" path={path} sessionId={sessionId} onToast={() => {}} />} />
    </HashRouter>
  ));
}

function mountDesktop(path = '/home/u/proj/src', sessionId?: string) {
  setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
  Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <GitSection hostId="h1" path={path} sessionId={sessionId} onToast={() => {}} />} />
    </HashRouter>
  ));
}

/** A group head label ("Changes 3") — scoped so the tab button never matches. */
const groupLabel = (name: string) =>
  screen.queryAllByText((content, el) => content === name && !!el?.classList.contains('section-label') && !!el.closest('.git-group-head'));

/** Row lookup over the live DOM (robust against testing-library scoping). */
const rows = () => [...document.querySelectorAll<HTMLElement>('[data-testid="git-row"]')];
const rowOf = (path: string) => rows().find((r) => (r.dataset.gitKey ?? '').endsWith(`:${path}`) || r.dataset.gitKey === path);

describe('Git section (WP-A15) — phone', () => {
  it('shows the repo line and groups with +/- counts; the source select offers ZCode sources', async () => {
    mountPhone();
    expect(await screen.findByText('proj')).toBeTruthy();
    expect(screen.getByText('main · origin/main ↑2 ↓1')).toBeTruthy();
    const select = screen.getByTestId('git-source') as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(['unstaged', 'staged', 'branch']);
    // Unstaged source: the unipi grouping without the Staged group.
    await waitFor(() => expect(groupLabel('Changes').length).toBeGreaterThan(0));
    expect(groupLabel('Untracked').length).toBeGreaterThan(0);
    expect(groupLabel('Staged').length).toBe(0);
    expect(rowOf('b.txt')).toBeTruthy();
    expect(rowOf('new.md')).toBeTruthy();
    expect(rowOf('src/a.ts')).toBeUndefined();
    // The Staged source shows it.
    fireEvent.change(select, { target: { value: 'staged' } });
    await waitFor(() => expect(rowOf('src/a.ts')).toBeTruthy());
    expect(groupLabel('Staged').length).toBeGreaterThan(0);
    expect(rowOf('b.txt')).toBeUndefined();
  });

  it('stage toggle sends stage/unstage and moves the row between sources', async () => {
    mountPhone();
    await screen.findAllByTestId('git-row');
    const row = rows().find((r) => r.dataset.gitKey === 'untracked:new.md')!;
    fireEvent.click(within(row).getByRole('button', { name: /stage new\.md/i }));
    await waitFor(() => expect(gitOps().some((c) => c.action === 'stage' && (c.paths as string[])[0] === 'new.md')).toBe(true));
    // The Staged source now has it.
    fireEvent.change(screen.getByTestId('git-source'), { target: { value: 'staged' } });
    await waitFor(() => expect(rowOf('new.md')).toBeTruthy());
    fireEvent.click(within(rowOf('new.md')!).getByRole('button', { name: /unstage new\.md/i }));
    await waitFor(() => expect(gitOps().some((c) => c.action === 'unstage')).toBe(true));
  });

  it('commit composer: disabled until a message, commits, clears', async () => {
    mountPhone();
    const composer = await screen.findByTestId('git-composer');
    const btn = within(composer).getByRole('button', { name: /commit/i });
    expect((btn as HTMLButtonElement).disabled).toBe(true);
    fireEvent.input(within(composer).getByLabelText('Commit message'), { target: { value: 'Fix the build' } });
    await waitFor(() => expect((within(composer).getByRole('button', { name: /commit/i }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.submit(screen.getByTestId('git-composer'));
    await waitFor(() => expect(gitOps().some((c) => c.action === 'commit' && c.message === 'Fix the build')).toBe(true));
  });

  it('tap a file → the diff layer; the Branch and Last-turn sources carry their diffs', async () => {
    mountPhone();
    await screen.findAllByTestId('git-row');
    fireEvent.click(rowOf('b.txt')!);
    expect(await screen.findByTestId('git-detail-layer')).toBeTruthy();
    expect(screen.getByTestId('git-diff')).toBeTruthy();
    expect(gitOps().some((c) => c.action === 'diff' && c.file === 'b.txt')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'Close diff' }));
    await waitFor(() => expect(screen.queryByTestId('git-detail-layer')).toBeNull());

    // Branch source: one round trip, rows carry diffs (no per-file diff fetch).
    const before = gitOps().length;
    fireEvent.change(screen.getByTestId('git-source'), { target: { value: 'branch' } });
    await waitFor(() => expect(groupLabel('vs origin/main').length).toBeGreaterThan(0));
    expect(gitOps().slice(before).some((c) => c.action === 'branch_files')).toBe(true);
    fireEvent.click(rows().find((r) => r.dataset.gitKey === 'staged:src/a.ts')!);
    expect(await screen.findByTestId('git-diff')).toBeTruthy();
    expect(gitOps().slice(before).some((c) => c.action === 'diff')).toBe(false);
  });

  it('the Last turn source lists the last turn\'s tool files (turn_files op)', async () => {
    mountPhone('/home/u/proj', 's1');
    await screen.findAllByTestId('git-row');
    const select = screen.getByTestId('git-source') as HTMLSelectElement;
    expect([...select.options].map((o) => o.value)).toEqual(['unstaged', 'staged', 'branch', 'turn']);
    fireEvent.change(select, { target: { value: 'turn' } });
    await waitFor(() => expect(gitOps().some((c) => c.action === 'turn_files' && JSON.stringify(c.files) === JSON.stringify(['src/a.ts', 'b.txt']))).toBe(true));
    await waitFor(() => expect(rowOf('b.txt')).toBeTruthy());
  });

  it('discard asks first, then sends discard with confirm', async () => {
    mountPhone();
    fireEvent.contextMenu((await screen.findAllByTestId('git-row'))[0]!);
    fireEvent.click(await screen.findByRole('menuitem', { name: /discard changes/i }));
    fireEvent.click(await screen.findByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(gitOps().some((c) => c.action === 'discard' && c.confirm === true)).toBe(true));
  });

  it('history: pages of commits, load more, open a commit', async () => {
    mountPhone();
    await screen.findAllByTestId('git-row');
    fireEvent.click(screen.getByRole('tab', { name: /history/i }));
    expect(await screen.findAllByTestId('git-commit-row')).toHaveLength(50);
    fireEvent.click(screen.getByRole('button', { name: /load more/i }));
    await waitFor(() => expect(screen.getAllByTestId('git-commit-row')).toHaveLength(60));
    fireEvent.click(screen.getAllByTestId('git-commit-row')[0]!);
    expect(await screen.findByTestId('git-commit')).toBeTruthy();
  });

  it('not a repo → empty state; clean tree → nothing to commit', async () => {
    overrides.repo = () => ({ ok: true, value: { repo: false, detached: false, remotes: [], ahead: 0, behind: 0, stashes: 0 } });
    mountPhone();
    expect(await screen.findByTestId('git-not-repo')).toBeTruthy();
  });

  it('row menu → Reveal in tree opens the Files layer (phone)', async () => {
    mountPhone();
    fireEvent.contextMenu((await screen.findAllByTestId('git-row'))[0]!);
    fireEvent.click(await screen.findByRole('menuitem', { name: /reveal in tree/i }));
    await waitFor(() => expect(location.hash).toBe('#/files'));
  });

  it('row menu → Open in editor sets the editor target at the repo path (phone)', async () => {
    mountPhone();
    fireEvent.contextMenu((await screen.findAllByTestId('git-row'))[0]!);
    fireEvent.click(await screen.findByRole('menuitem', { name: /open in editor/i }));
    expect(getEditorTarget()?.path).toBe('/home/u/proj/b.txt');
  });
});

describe('Git section (WP-A15) — desktop pane', () => {
  it('row click expands the diff inline (lazy), Branch rows come preloaded', async () => {
    mountDesktop();
    fireEvent.click((await screen.findAllByTestId('git-row'))[0]!);
    expect(await screen.findByTestId('git-diff')).toBeTruthy();
    expect(screen.queryByTestId('git-detail-layer')).toBeNull();
  });

  it('find filters rows and reports the match count', async () => {
    mountDesktop();
    await screen.findAllByTestId('git-row');
    fireEvent.click(screen.getByRole('button', { name: 'Find in changes' }));
    const find = screen.getByTestId('git-find');
    fireEvent.input(find, { target: { value: 'b.t' } });
    await waitFor(() => expect(screen.getByText('1 match')).toBeTruthy());
    expect(rowOf('new.md')).toBeUndefined();
    expect(rowOf('b.txt')).toBeTruthy();
  });

  it('Ctrl+Enter commits from the message box', async () => {
    mountDesktop();
    await screen.findByTestId('git-composer');
    fireEvent.input(screen.getByLabelText('Commit message'), { target: { value: 'Desktop commit' } });
    fireEvent.keyDown(window, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(gitOps().some((c) => c.action === 'commit' && c.message === 'Desktop commit')).toBe(true));
  });
});

describe('Git commit key inside the message box (D-39)', () => {
  it('Ctrl+Enter pressed in the focused message textarea commits', async () => {
    mountDesktop();
    await screen.findByTestId('git-composer');
    const box = screen.getByLabelText('Commit message');
    fireEvent.input(box, { target: { value: 'Typed commit' } });
    fireEvent.keyDown(box, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(gitOps().some((c) => c.action === 'commit' && c.message === 'Typed commit')).toBe(true));
  });
});

describe('Git auto-refresh (Z-98)', () => {
  it('watches the repo root; one git_changed → exactly one extra status refresh; unmount stops the watch', async () => {
    mountDesktop();
    await screen.findAllByTestId('git-row');
    await waitFor(() => expect(calls.some((c) => c.op === 'git_watch' && (c.args as { root: string }).root === '/home/u/proj' && (c.args as { on: boolean }).on === true)).toBe(true));
    const before = statusCalls().length;
    eventSinks.forEach((sink) => sink({ kind: 'git_changed', root: '/home/u/proj' } as HubEvent));
    await waitFor(() => expect(statusCalls().length).toBeGreaterThan(before));
    expect(statusCalls().length - before).toBe(1);
    // A change under another root does not refresh.
    const other = statusCalls().length;
    eventSinks.forEach((sink) => sink({ kind: 'git_changed', root: '/elsewhere' } as HubEvent));
    await new Promise((r) => setTimeout(r, 150));
    expect(statusCalls().length).toBe(other);
    // Unmount: the watch reference is released.
    cleanup();
    await waitFor(() => expect(calls.filter((c) => c.op === 'git_watch').some((c) => (c.args as { on: boolean }).on === false)).toBe(true));
  });
});

describe('StatusPaneLayer (phone §3.9)', () => {
  it('renders the section full screen from gitTarget and closes on Back', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: false });
    openGit({ hostId: 'h1', path: '/home/u/proj', from: 'files' });
    render(() => (
      <HashRouter>
        <Route path="*" component={() => <StatusPaneLayer />} />
      </HashRouter>
    ));
    expect(await screen.findByTestId('git-section')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Close Git' }));
    await waitFor(() => expect(screen.queryByTestId('git-section')).toBeNull());
  });
});

describe('Git auto-refresh over the MockHost (Z-98, acceptance)', () => {
  const scenario: MockScenario = {
    name: 'a15-watch',
    hosts: [
      {
        id: 'h1',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [{ session_id: 's1', name: 'Repo', cwd: '/home/u/proj', run_state: 'idle', updated_at: 1, message_count: 2 }],
        git: {
          root: '/home/u/proj',
          branch: 'main',
          files: [{ path: 'src/a.ts', area: 'unstaged', status: 'M', add: 2, del: 1 }],
          commits: [{ hash: 'abc1234def', subject: 'Initial commit' }],
        },
      },
    ],
  };

  it('one MockHost git_changed → exactly one git status refresh', async () => {
    const world = createMockHost(scenario);
    setHostClientFactory(world.client as never);
    mountDesktop('/home/u/proj');
    await screen.findAllByTestId('git-row');
    const host = world.client('h1');
    await waitFor(() => expect(host.watched.has('/home/u/proj')).toBe(true));
    const status = () => world.log.filter((e) => e.kind === 'request' && e.op === 'git_op' && (e.args as { cmd?: { action?: string } }).cmd?.action === 'status').length;
    const before = status();
    host.gitChanged();
    await waitFor(() => expect(status()).toBe(before + 1));
    await new Promise((r) => setTimeout(r, 120));
    expect(status()).toBe(before + 1);
    // Unmount releases the watch reference.
    cleanup();
    await waitFor(() => expect(host.watched.has('/home/u/proj')).toBe(false));
    const log = world.log.filter((e) => e.kind === 'request' && e.op === 'git_watch');
    expect(log.some((e) => (e.args as { on?: boolean }).on === false)).toBe(true);
  });
});

describe('chat-vanish: the status pane pins the SHOWN chat only', () => {
  it('a session switch moves the pin (no stale pin on the first chat)', async () => {
    const { sessions } = await import('../../lib/session/store');
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: false });
    const [sid, setSid] = createSignal('s1');
    render(() => (
      <HashRouter>
        <Route path="*" component={() => <GitSection hostId="h1" path="/home/u/proj" sessionId={sid()} onToast={() => {}} />} />
      </HashRouter>
    ));
    const refs = (id: string) => (sessions().peek('h1', id) as unknown as { refs: number } | undefined)?.refs ?? 0;
    await waitFor(() => expect(refs('s1')).toBe(1));
    setSid('s2');
    await waitFor(() => expect(refs('s2')).toBe(1));
    expect(refs('s1')).toBe(0);
  });
});
