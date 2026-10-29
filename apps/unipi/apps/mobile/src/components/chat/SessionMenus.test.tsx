// @vitest-environment jsdom
/**
 * Resume and Session tree sheets (UNI-158): basics — they load their list
 * when opened, show the current session/leaf marked, and call back on pick.
 * BoardSheet portals its content to document.body, so queries go through
 * `screen` (document.body), not the render container.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { ChatHeaderMenu, ResumeSheet, TreeSheet, filterTree, flattenTree, relativeTimeMs } from './SessionMenus';
import type { Chat, SessionsItem, TreeNode } from '../../lib/chat/store';

afterEach(() => (document.body.innerHTML = ''));

function fakeChat(overrides: Partial<Chat> = {}): Chat {
  const [state] = createStore({
    conn: 'live',
    run: { running: false },
    entries: [],
    truncated: false,
    tools: {},
    commands: [],
    models: [],
    dialogs: [],
    queue: [],
    pending: [],
    hasOlder: false,
    loadingOlder: false,
    seams: [],
  });
  return { state, ...overrides } as unknown as Chat;
}

describe('relativeTimeMs', () => {
  it('formats ms-since-epoch deltas', () => {
    const now = 1_000_000;
    expect(relativeTimeMs(now - 10_000, now)).toBe('just now');
    expect(relativeTimeMs(now - 90_000, now)).toBe('2m ago');
    expect(relativeTimeMs(undefined, now)).toBe('');
  });
});

describe('ResumeSheet', () => {
  it('loads sessions when opened and marks the current one', async () => {
    const items: SessionsItem[] = [
      { path: '/a.jsonl', id: 'a', cwd: '/p', firstMessage: 'fix the bug', modified: Date.now(), messageCount: 3, current: true },
      { path: '/b.jsonl', id: 'b', cwd: '/p', firstMessage: 'add a feature', modified: Date.now() - 60_000, messageCount: 1, current: false },
    ];
    const sessions = vi.fn(async () => ({ items, more: false }));
    const chat = fakeChat({ sessions } as never);
    const onPick = vi.fn();
    render(() => <ResumeSheet open chat={chat} onClose={() => {}} onPick={onPick} />);
    await waitFor(() => expect(sessions).toHaveBeenCalledWith('cwd', undefined));
    await waitFor(() => expect(screen.getByText('fix the bug')).toBeTruthy());
    const current = document.body.querySelector('[aria-current="true"]');
    expect(current?.textContent).toContain('fix the bug');
    const rows = document.body.querySelectorAll('.resume-row');
    expect(rows).toHaveLength(2);
    (rows[1] as HTMLButtonElement).click();
    expect(onPick).toHaveBeenCalledWith(items[1]);
  });

  it('switches scope to All projects and re-queries', async () => {
    const sessions = vi.fn(async () => ({ items: [], more: false }));
    const chat = fakeChat({ sessions } as never);
    render(() => <ResumeSheet open chat={chat} onClose={() => {}} onPick={() => {}} />);
    await waitFor(() => expect(sessions).toHaveBeenCalledWith('cwd', undefined));
    screen.getByText('All projects').click();
    await waitFor(() => expect(sessions).toHaveBeenCalledWith('all', undefined));
  });
});

describe('ChatHeaderMenu', () => {
  const anchor = { top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;

  it('"New chat in this workspace" (a separate pi beside this one) shows with onNewHere and runs it; "New session" stays', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('New chat in this workspace')).toBeNull();
    cleanup();
    const onNewHere = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={onDismiss} onNew={() => {}} onNewHere={onNewHere} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.getByText('New session')).toBeTruthy();
    screen.getByText('New chat in this workspace').click();
    expect(onDismiss).toHaveBeenCalled();
    expect(onNewHere).toHaveBeenCalled();
  });

  it('shows "Files here" only when onFilesHere is provided (a known cwd)', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Files here')).toBeNull();
  });

  it('calls onFilesHere when the row is tapped', () => {
    const onFilesHere = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename onFilesHere={onFilesHere} />
    ));
    screen.getByText('Files here').click();
    expect(onFilesHere).toHaveBeenCalled();
  });

  it('UNI-217: "Git" shows only with onGit (a known cwd) and runs it', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Git')).toBeNull();
    cleanup();
    const onGit = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={onDismiss} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename onGit={onGit} />
    ));
    screen.getByText('Git').click();
    expect(onDismiss).toHaveBeenCalled();
    expect(onGit).toHaveBeenCalled();
  });

  it('UNI-188: shows Running / Context / Session info only with onOpenSection, with the running count hint', () => {
    render(() => (
      <ChatHeaderMenu anchor={anchor} onDismiss={() => {}} onNew={() => {}} onResume={() => {}} onTree={() => {}} onRename={() => {}} canRename />
    ));
    expect(screen.queryByText('Running')).toBeNull();

    cleanup();
    const onOpenSection = vi.fn();
    const onDismiss = vi.fn();
    render(() => (
      <ChatHeaderMenu
        anchor={anchor}
        onDismiss={onDismiss}
        onNew={() => {}}
        onResume={() => {}}
        onTree={() => {}}
        onRename={() => {}}
        canRename
        onOpenSection={onOpenSection}
        runningCount={2}
      />
    ));
    expect(screen.getByText('Running').parentElement?.textContent).toContain('2');
    screen.getByText('Running').click();
    expect(onOpenSection).toHaveBeenCalledWith('running');
    expect(onDismiss).toHaveBeenCalled();
    screen.getByText('Context').click();
    expect(onOpenSection).toHaveBeenCalledWith('context');
    screen.getByText('Session info').click();
    expect(onOpenSection).toHaveBeenCalledWith('info');
  });
});

describe('TreeSheet', () => {
  it('loads the tree when opened and marks the current leaf', async () => {
    const nodes: TreeNode[] = [
      { id: 'm1', parentId: null, kind: 'user', preview: 'hello', onPath: true, current: false },
      { id: 'm2', parentId: 'm1', kind: 'assistant', preview: 'hi!', onPath: true, current: true },
    ];
    const tree = vi.fn(async () => nodes);
    const chat = fakeChat({ tree } as never);
    const onPick = vi.fn();
    render(() => <TreeSheet open chat={chat} onClose={() => {}} onPick={onPick} />);
    await waitFor(() => expect(tree).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText('hi!')).toBeTruthy());
    const current = document.body.querySelector('[aria-current="true"]');
    expect(current?.textContent).toContain('hi!');
    const rows = document.body.querySelectorAll('.tree-row');
    expect(rows).toHaveLength(2);
    (rows[0] as HTMLButtonElement).click();
    expect(onPick).toHaveBeenCalledWith(nodes[0], true);
  });

  it('UNI-275: with onNavigate, a point on the current branch offers "Go to this point" (keeps the chat) or "Start a new branch here"', async () => {
    const nodes: TreeNode[] = [
      { id: 'm1', parentId: null, kind: 'user', preview: 'hello', onPath: true, current: false },
      { id: 'm2', parentId: 'm1', kind: 'assistant', preview: 'hi!', onPath: true, current: true },
      { id: 'x1', parentId: 'm1', kind: 'assistant', preview: 'other branch', onPath: false, current: false },
    ];
    const chat = fakeChat({ tree: vi.fn(async () => nodes) } as never);
    const onPick = vi.fn();
    const onNavigate = vi.fn();
    render(() => <TreeSheet open chat={chat} onClose={() => {}} onPick={onPick} onNavigate={onNavigate} />);
    await waitFor(() => expect(screen.getByText('hello')).toBeTruthy());
    fireEvent.click(screen.getByText('hello').closest('button')!);
    fireEvent.click(await screen.findByText('Go to this point'));
    expect(onNavigate).toHaveBeenCalledWith(nodes[0]);
    expect(onPick).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('hello').closest('button')!);
    fireEvent.click(await screen.findByText('Start a new branch here'));
    expect(onPick).toHaveBeenCalledWith(nodes[0], true);
    // Another branch: nothing on screen to go to — tapping switches to it.
    fireEvent.click(screen.getByText('other branch').closest('button')!);
    expect(onPick).toHaveBeenLastCalledWith(nodes[2], true);
  });
});

describe('flattenTree (UNI-186, pi /tree layout)', () => {
  function node(id: string, parentId: string | null, current = false): TreeNode {
    return { id, parentId, kind: 'message', preview: id, onPath: false, current };
  }
  const view = (rows: ReturnType<typeof flattenTree>) => rows.map((r) => `${r.cells.map((c) => c || ' ').join('')}${r.node.id}`);

  it('a linear chain of messages stays flat', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'b'), node('d', 'c')]);
    expect(rows.map((r) => r.depth)).toEqual([0, 0, 0, 0]);
  });

  it('a fork steps in with connectors and the first generation after it steps in again', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'a'), node('d', 'b'), node('e', 'd')]);
    expect(view(rows)).toEqual(['a', '├b', '│ d', '│ e', '└c']);
  });

  it('the branch holding the current leaf comes first among siblings', () => {
    const rows = flattenTree([node('a', null), node('b', 'a'), node('c', 'a'), node('d', 'c', true)]);
    expect(rows.map((r) => r.node.id)).toEqual(['a', 'c', 'd', 'b']);
    expect(view(rows)[1]).toBe('├c');
  });

  it('several roots hang off a virtual root and their chains indent once', () => {
    const rows = flattenTree([node('r1', null), node('x', 'r1'), node('r2', null), node('y', 'r2')]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([
      ['r1', 0],
      ['x', 1],
      ['r2', 0],
      ['y', 1],
    ]);
  });

  it('survives a malformed cycle without hanging', () => {
    expect(() => flattenTree([node('a', 'b'), node('b', 'a')])).not.toThrow();
  });
});

describe('filterTree (UNI-194, pi /tree filters)', () => {
  const n = (id: string, parentId: string | null, kind: string, extra: Partial<TreeNode> = {}): TreeNode => ({ id, parentId, kind, preview: id, onPath: false, current: false, ...extra });
  const nodes = [
    n('u1', null, 'user'),
    n('a1', 'u1', 'assistant', { preview: '' }),
    n('t1', 'a1', 'toolResult'),
    n('m1', 't1', 'model_change'),
    n('a2', 'm1', 'assistant', { label: 'checkpoint' }),
    n('u2', 'a2', 'user', { current: true }),
  ];
  const ids = (list: TreeNode[]) => list.map((x) => `${x.id}<${x.parentId ?? '-'}`);

  it('default hides settings entries and text-less assistant rows, re-parenting', () => {
    expect(ids(filterTree(nodes, 'default'))).toEqual(['u1<-', 't1<u1', 'a2<t1', 'u2<a2']);
  });
  it('no-tools also hides tool results', () => {
    expect(ids(filterTree(nodes, 'no-tools'))).toEqual(['u1<-', 'a2<u1', 'u2<a2']);
  });
  it('user-only keeps user messages (and the current leaf)', () => {
    expect(ids(filterTree(nodes, 'user-only'))).toEqual(['u1<-', 'u2<u1']);
  });
  it('labeled-only keeps labeled entries plus the current leaf', () => {
    expect(ids(filterTree(nodes, 'labeled-only'))).toEqual(['a2<-', 'u2<a2']);
  });
  it('all shows everything', () => {
    expect(filterTree(nodes, 'all')).toHaveLength(6);
  });
  it('the sheet filters with a chip and remembers it', async () => {
    localStorage.removeItem('unipi.chat.treeFilter');
    const chat = fakeChat({ tree: vi.fn(async () => nodes) } as never);
    render(() => <TreeSheet open chat={chat} onClose={() => {}} onPick={() => {}} />);
    await waitFor(() => expect(document.body.querySelectorAll('.tree-row')).toHaveLength(4));
    screen.getByRole('radio', { name: 'User' }).click();
    await waitFor(() => expect(document.body.querySelectorAll('.tree-row')).toHaveLength(2));
    expect(localStorage.getItem('unipi.chat.treeFilter')).toBe('user-only');
  });
});
