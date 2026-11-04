// @vitest-environment jsdom
/**
 * Session side tabs (WP-A10 acceptance): each tab opens from its entry
 * point, is tagged with its session (persisted per project, hidden while
 * another session shows, dropped with the session), the side chat **never
 * sends to the main session** (MockHost log), the subagent tab follows the
 * child's new messages, and the phone opens the same views as a sheet (tree /
 * side chat / subagent) or a full-screen layer (diff).
 *
 * The pure tree logic is in `tree.test.ts`; the split-diff pager in
 * `DiffTab.test.tsx`.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import type { JSX } from 'solid-js';
import { createMockHost, type MockScenario, type MockSession } from '../../../dev/mock/host';
import { setHostClientFactory } from '../../../lib/host/factory';
import { createSession, resetSessions, sessions, type SessionHandle } from '../../../lib/session/store';
import { panes, resetPanes } from '../../../lib/panes';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { resetComposerDrafts, composerDraft } from '../../../lib/composer/drafts';
import { clearEditorTarget } from '../../../lib/editorTarget';
import { clearExpansion } from '../../../lib/timeline/expansion';
import { BtwTab } from './BtwTab';
import { agentOfEntry, agentOfStepData } from './agentEntry';
import { EditBody } from '../../timeline/tools/bodies';
import { StepRow } from '../../timeline/tools/StepRow';
import { openBtwTab, openDiffTab, openSubagentTab, openTreeTab, phoneSessionTab, closePhoneSessionTab, seedPhoneSessionTab } from './sessionTabs';
import { SessionTabLayer } from './SessionTabLayer';
import { SubagentTab } from './SubagentTab';
import { TreeTab } from './TreeTab';
import type { PaneTabProps } from '../../../lib/panes/kinds';
import type { SessionEntry } from '../../../lib/session/types';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-one';
const CWD = '/home/you/unipi';
const S = 's-tabs';
const SCOPE = { hostId: HOST, projectKey: CWD };

const part = (text: string) => ({ type: 'text', text });

/** The chat in the flat wire form the host sends ([tree-flat]): nodes linked by `entry.parentId`. */
const flatTree = () => [
  { entry: { type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: [part('Fix the failing test')] } } as SessionEntry },
  { entry: { type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: [part('Ran the suite')] } } as SessionEntry },
  { entry: { type: 'message', id: 'u2', parentId: 'a1', message: { role: 'user', content: [part('Also check cli')] } } as SessionEntry },
  { entry: { type: 'message', id: 'b1', parentId: 'u1', message: { role: 'assistant', content: [part('Tried the other branch')] } } as SessionEntry },
];

const ENTRIES: SessionEntry[] = [
  { type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: [part('Fix the failing test')] } },
  {
    type: 'message',
    id: 'a1',
    parentId: 'u1',
    message: {
      role: 'assistant',
      content: [{ type: 'toolCall', id: 'tc-edit', name: 'edit', arguments: { path: 'src/math.ts', edits: [{ oldText: 'return a - b;', newText: 'return a + b;' }] } }],
      stopReason: 'toolUse',
    },
  },
  { type: 'message', id: 'r1', parentId: 'a1', message: { role: 'toolResult', toolCallId: 'tc-edit', toolName: 'edit', content: [part('Replaced 1 block.')], details: { firstChangedLine: 7 } } },
];

let btWAnswer = 'It is a Solid + Tauri app.';
const asked: string[] = [];
let messages = [{ role: 'user', content: [part('Map the status topics')] }, { role: 'assistant', content: [part('Reading src/lib/status/lh.ts')] }];

function session(): MockSession {
  const cmd: NonNullable<MockSession['cmd']> = {
    // The host's flat wire form ([tree-flat]); the nested one is covered by tree.test.ts.
    get_tree: () => ({ tree: flatTree(), leafId: 'u2' }),
    // [chat-tree] The branch page a tree jump reloads.
    get_entries: () => ({ entries: ENTRIES, leafId: 'r1', total: ENTRIES.length, hasMore: false }),
    '_oino/navigate_tree': (c) => ({ editorText: `> Fix the failing test\n> go to ${String((c as { targetId?: string }).targetId)}` }),
    '_oino/service_call': (c) => {
      const name = String((c as { name?: string }).name ?? '');
      if (name === 'btw.ask') {
        asked.push(String(((c as { payload?: { question?: string } }).payload ?? {}).question ?? ''));
        return { id: `p${asked.length}`, answer: btWAnswer, done: true };
      }
      if (name === 'btw.list') return asked.map((q, i) => ({ id: `p${i + 1}`, question: q, answer: btWAnswer, done: true }));
      return {};
    },
    '_oino/agents': () => ({ agents: [{ id: 'ag-1', label: 'Subagent · map topics', owner: 'unipi.subagents', role: 'subagent_explore', state: 'running', depth: 1 }] }),
    '_oino/agent_messages': () => ({ messages }),
    '_oino/agent_abort': () => ({}),
  };
  return {
    snapshot: {
      session_id: S,
      reset: true,
      seq: 0,
      state: { sessionId: S, isStreaming: false, messageCount: ENTRIES.length },
      entries: ENTRIES,
      leaf_id: 'r1',
      commands: [],
      ui: { scope: 'session', sessionId: S, rev: 0, entries: [], dialogs: [] },
      topics: [],
    },
    cmd,
  };
}

const scenario = (): MockScenario => ({
  name: 'tabs-test',
  hosts: [{ id: HOST, name: 'coffee', conn: { state: 'connected' }, cards: [{ session_id: S, title: 'Tabs', cwd: CWD, last_activity: 1 }] as never[], sessions: { [S]: session() } }],
});

let world: ReturnType<typeof createMockHost>;

beforeEach(() => {
  asked.length = 0;
  btWAnswer = 'It is a Solid + Tauri app.';
  messages = [{ role: 'user', content: [part('Map the status topics')] }, { role: 'assistant', content: [part('Reading src/lib/status/lh.ts')] }];
  world = createMockHost(scenario());
  setHostClientFactory(world.client);
  localStorage.clear();
  resetPanes();
  resetSessions();
  resetLayout();
  setDesktopOverride(true);
  clearExpansion();
  resetComposerDrafts();
  clearEditorTarget();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  setDesktopOverride(undefined);
  vi.restoreAllMocks();
});

async function openHandle(): Promise<SessionHandle> {
  const handle = createSession(HOST, S, { client: world.client(HOST) });
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  await new Promise((r) => setTimeout(r, 10));
  return handle;
}

const tab = (kind: string, params: Record<string, unknown>): PaneTabProps => ({
  tab: { id: `t-${kind}`, kind, params: { hostId: HOST, sessionId: S, ...params }, sessionId: S },
  area: 'side',
  hostId: HOST,
  projectKey: CWD,
  sessionId: S,
  close: () => {},
  setTitle: () => {},
});

/** Renders with the router (DiffTab's "Open in editor" navigates on the phone). */
function withRouter(body: () => JSX.Element) {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => body()} />
    </HashRouter>
  ));
}

describe('side tabs entry points (Z-40, Z-18, Z-31, Z-42)', () => {
  it('"View diff" opens the diff tab for that tool call', async () => {
    await import('./DiffTab');
    openDiffTab({ hostId: HOST, sessionId: S, cwd: CWD }, { toolId: 'tc-edit', path: 'src/math.ts' });
    const tabs = () => panes().visible(SCOPE, 'side', { sessionId: S });
    expect(tabs().map((t) => t.kind)).toEqual(['diff']);
    expect(tabs()[0]!.params).toMatchObject({ toolId: 'tc-edit', path: 'src/math.ts', hostId: HOST, sessionId: S });
    expect(tabs()[0]!.sessionId).toBe(S);
  });

  it('Tree, Side chat and Subagent open their tabs (session-tagged)', async () => {
    await import('./TreeTab');
    await import('./BtwTab');
    await import('./SubagentTab');
    openTreeTab({ hostId: HOST, sessionId: S, cwd: CWD });
    openBtwTab({ hostId: HOST, sessionId: S, cwd: CWD }, 'why so slow?');
    openSubagentTab({ hostId: HOST, sessionId: S, cwd: CWD }, { agentId: 'ag-1', name: 'Subagent · map topics' });
    const tabs = panes().visible(SCOPE, 'side', { sessionId: S });
    expect(tabs.map((t) => t.kind)).toEqual(['tree', 'btw', 'subagent']);
    expect(tabs.find((t) => t.kind === 'btw')!.params).toMatchObject({ question: 'why so slow?' });
    expect(tabs.find((t) => t.kind === 'subagent')!.params).toMatchObject({ agentId: 'ag-1' });
    expect(tabs.every((t) => t.sessionId === S)).toBe(true);
  });

  it('a tab persists per session: hidden for another session, dropped with it', async () => {
    openTreeTab({ hostId: HOST, sessionId: S, cwd: CWD });
    expect(panes().visible(SCOPE, 'side', { sessionId: S })).toHaveLength(1);
    // Another session of the same project does not see it (Z-5)…
    expect(panes().visible(SCOPE, 'side', { sessionId: 's-other' })).toHaveLength(0);
    // …and it comes back when its session shows again.
    expect(panes().visible(SCOPE, 'side', { sessionId: S })).toHaveLength(1);
    // Closing the session drops its tabs (acceptance: closes with the session).
    panes().dropSession(HOST, S);
    expect(panes().visible(SCOPE, 'side', { sessionId: S })).toHaveLength(0);
  });

  it('the `+` launcher offers Tree and Side chat once their views are plugged in', async () => {
    await import('./TreeTab');
    await import('./BtwTab');
    const { launchableKinds } = await import('../../../lib/panes/kinds');
    const labels = launchableKinds('side', true).map((k) => k.launcher!.label);
    expect(labels).toContain('Tree');
    expect(labels).toContain('Side chat');
  });
});

describe('timeline entry points (Z-40, Z-42)', () => {
  const ctx = async () => {
    const handle = await openHandle();
    return {
      session: handle,
      hostId: HOST,
      sessionId: () => S,
      cwd: () => CWD,
      refs: {} as never,
      commands: () => new Set<string>(),
      columnWidth: () => 720,
    } as unknown as import('../../timeline/context').TimelineCtx;
  };

  it('an edit tool body offers "View diff" → the diff tab for that call', async () => {
    await import('./DiffTab');
    const c = await ctx();
    render(() => (
      <EditBody
        tool={{ id: 'tc-edit', name: 'edit', args: { path: 'src/math.ts', edits: [{ oldText: 'a', newText: 'b' }] }, state: 'ok' }}
        ctx={c}
      />
    ));
    fireEvent.click(screen.getByTestId('tool-view-diff'));
    const tab = panes().visible(SCOPE, 'side', { sessionId: S }).find((t) => t.kind === 'diff');
    expect(tab?.params).toMatchObject({ toolId: 'tc-edit', path: 'src/math.ts' });
  });

  it('a subagent step row opens the child transcript tab', async () => {
    await import('./SubagentTab');
    const c = await ctx();
    render(() => <StepRow entry={{ type: 'custom', id: 'st1', customType: 'subagent-step', data: { agentId: 'ag-1', title: 'Map topics' } }} ctx={c} />);
    fireEvent.click(screen.getByTestId('tl-step-open'));
    const tab = panes().visible(SCOPE, 'side', { sessionId: S }).find((t) => t.kind === 'subagent');
    expect(tab?.params).toMatchObject({ agentId: 'ag-1' });
  });

  it('reads the agent of a completion card and a step entry', async () => {
    expect(agentOfStepData({ agentId: 'ag-1', title: 'Map topics' })).toEqual({ agentId: 'ag-1', name: 'Map topics' });
    const card = {
      type: 'custom_message',
      id: 'cm1',
      customType: 'subagent-completion',
      message: { role: 'custom', details: { owner: 'subagents', id: 'ag-2', title: 'Docs', status: 'completed' } },
    } as never;
    expect(agentOfEntry(card)).toMatchObject({ agentId: 'ag-2', name: 'Docs', owner: 'subagents' });
    // An unrelated card carries no agent.
    expect(agentOfEntry({ type: 'custom_message', id: 'x', customType: 'acme:report', message: { role: 'custom', details: { id: 'nope' } } } as never)).toBeUndefined();
  });
});

describe('tree tab (Z-18)', () => {
  it('lists the branches, filters, and a jump restores the message into the composer', async () => {
    const handle = await openHandle();
    withRouter(() => <TreeTab {...tab('tree', {})} />);
    await waitFor(() => expect(screen.getByTestId('tree-tab')).toBeTruthy());
    await waitFor(() => expect(screen.getByText('Fix the failing test')).toBeTruthy());
    expect(screen.getByText('Tried the other branch')).toBeTruthy();
    // The current leaf is marked.
    const current = document.querySelector('[data-current]');
    expect(current?.textContent).toContain('Also check cli');
    // Filters: user only leaves the two user messages.
    screen.getByRole('radio', { name: 'User' }).click();
    await waitFor(() => expect(document.querySelectorAll('.ttab-row')).toHaveLength(2));
    // A point on the current branch jumps (the reply carries the message text).
    screen.getByRole('radio', { name: 'Default' }).click();
    await waitFor(() => expect(screen.getByText('Fix the failing test')).toBeTruthy());
    screen.getByText('Fix the failing test').closest('button')!.click();
    // On the current branch but not the leaf: the row asks which jump (Z-18).
    (await screen.findByText('Go to this point')).click();
    await waitFor(() => expect(composerDraft(handle.key).text).toContain('Fix the failing test'));
  });

  it('a jump reloads the chat\'s branch and the tree follows a jump made elsewhere (chat-tree)', async () => {
    await openHandle();
    withRouter(() => <TreeTab {...tab('tree', {})} />);
    await waitFor(() => expect(screen.getByText('Tried the other branch')).toBeTruthy());
    // The registry's handle: the one the tab and the chat share.
    const handle = sessions().ensure(HOST, S);
    const trees = () => world.log.filter((l) => l.kind === 'cmd' && l.op === 'get_tree').length;
    const pages = () => world.log.filter((l) => l.kind === 'cmd' && l.op === 'get_entries' && (l.args as { limit?: number }).limit === 80).length;
    const before = trees();
    // Off the current branch: one click navigates (pi /tree), the chat reloads its branch page.
    screen.getByText('Tried the other branch').closest('button')!.click();
    await waitFor(() => expect(world.log.some((l) => l.op === '_oino/navigate_tree' && (l.args as { targetId?: string }).targetId === 'b1')).toBe(true));
    await waitFor(() => expect(pages()).toBe(1));
    await waitFor(() => expect(trees()).toBe(before + 1));
    // "Return here" in the chat (or another window) moves the branch: the tree reloads once.
    await handle.navigate!('a1');
    await waitFor(() => expect(trees()).toBe(before + 2));
  });
});

describe('side chat tab (Z-31)', () => {
  it('asks through the btw service and never sends to the main session', async () => {
    await openHandle();
    withRouter(() => <BtwTab {...tab('btw', {})} />);
    await waitFor(() => expect(screen.getByTestId('btw-tab')).toBeTruthy());
    const field = screen.getByPlaceholderText('Ask a quick side question…') as HTMLTextAreaElement;
    field.value = 'why is the build slow?';
    field.dispatchEvent(new Event('input', { bubbles: true }));
    screen.getByTestId('btw-ask').click();
    await waitFor(() => expect(screen.getByTestId('btw-answer').textContent).toContain('Solid + Tauri'));
    expect(asked).toEqual(['why is the build slow?']);
    // The acceptance row: btw never touches the main session.
    const sessionCmds = world.log.filter((l) => l.kind === 'cmd');
    expect(sessionCmds.some((l) => l.op === 'prompt' || l.op === 'steer' || l.op === 'follow_up')).toBe(false);
    expect(sessionCmds.some((l) => l.op === '_oino/service_call' && (l.args as { name?: string }).name === 'btw.ask')).toBe(true);
  });

  it('"Ask in main chat" copies the Q&A into the session draft (still no prompt)', async () => {
    const handle = await openHandle();
    asked.push('why is the build slow?');
    const view = withRouter(() => <BtwTab {...tab('btw', {})} />);
    const button = await screen.findByTestId('btw-ask-main');
    button.click();
    await waitFor(() => expect(composerDraft(handle.key).text).toContain('why is the build slow?'));
    expect(world.log.some((l) => l.kind === 'cmd' && l.op === 'prompt')).toBe(false);
  });

  it('a seeded `/btw` question asks once on open', async () => {
    await openHandle();
    withRouter(() => <BtwTab {...tab('btw', { question: 'what is this repo?' })} />);
    await waitFor(() => expect(asked).toEqual(['what is this repo?']));
    await waitFor(() => expect(screen.getByTestId('btw-answer').textContent).toContain('Solid + Tauri'));
  });
});

describe('tree tab — failing host', () => {
  it('shows an error with Retry instead of loading forever', async () => {
    await openHandle();
    const handle = sessions().ensure(HOST, S);
    await waitFor(() => expect(handle.state.conn).toBe('live'));
    const real = handle.cmd.bind(handle);
    let fail = true;
    (handle as { cmd: unknown }).cmd = (c: { type: string }) => (fail && (c.type === 'get_tree' || c.type === 'get_entries') ? Promise.reject(new Error('boom')) : real(c as never));
    withRouter(() => <TreeTab {...tab('tree', {})} />);
    await waitFor(() => expect(screen.getByTestId('tree-error').textContent).toContain('boom'));
    fail = false;
    screen.getByText('Retry').click();
    await waitFor(() => expect(screen.getByText('Fix the failing test')).toBeTruthy());
  });

  it('shows a plain-object rejection message, never [object Object]', async () => {
    await openHandle();
    const handle = sessions().ensure(HOST, S);
    await waitFor(() => expect(handle.state.conn).toBe('live'));
    const real = handle.cmd.bind(handle);
    (handle as { cmd: unknown }).cmd = (c: { type: string }) =>
      c.type === 'get_tree' || c.type === 'get_entries'
        ? Promise.reject({ code: 'E_TIMEOUT', message: 'The session did not answer in time' })
        : real(c as never);
    withRouter(() => <TreeTab {...tab('tree', {})} />);
    const note = await screen.findByTestId('tree-error');
    expect(note.textContent).toContain('The session did not answer in time');
    expect(note.textContent).not.toContain('[object Object]');
  });

  it('past a long chat it mounts only the rows in view (windowed)', async () => {
    await openHandle();
    const handle = sessions().ensure(HOST, S);
    await waitFor(() => expect(handle.state.conn).toBe('live'));
    const long = Array.from({ length: 400 }, (_, i) => ({
      entry: { type: 'message', id: `e${i}`, parentId: i === 0 ? null : `e${i - 1}`, message: { role: 'user', content: [part(`turn ${i}`)] } } as SessionEntry,
    }));
    const real = handle.cmd.bind(handle);
    (handle as { cmd: unknown }).cmd = (c: { type: string }) => (c.type === 'get_tree' ? Promise.resolve({ tree: long, leafId: 'e399' }) : real(c as never));
    withRouter(() => <TreeTab {...tab('tree', {})} />);
    await waitFor(() => expect(document.querySelectorAll('.ui-virtual-row').length).toBeGreaterThan(0));
    expect(document.querySelectorAll('.ui-virtual-row').length).toBeLessThan(400);
    // The spacer claims the full 400 rows (40 px each); only a window is mounted.
    expect(document.querySelector('.ui-virtual')?.getAttribute('style')).toContain('16000px');
  });
});

describe('subagent tab (Z-42)', () => {
  it('shows the child transcript, its state, and follows new messages', async () => {
    await openHandle();
    withRouter(() => <SubagentTab {...tab('subagent', { agentId: 'ag-1' })} />);
    await waitFor(() => expect(screen.getByTestId('subagent-tab')).toBeTruthy());
    await waitFor(() => expect(screen.getByText('Reading src/lib/status/lh.ts')).toBeTruthy());
    expect(screen.getByText('Map the status topics')).toBeTruthy();
    expect(screen.getByTestId('stab-state').textContent).toBe('running');
    // The child keeps writing: the tab re-reads while it runs (live view).
    messages = [...messages, { role: 'assistant', content: [part('Found the publisher in LhSection')] }];
    await waitFor(() => expect(screen.getByText('Found the publisher in LhSection')).toBeTruthy(), { timeout: 5000 });
  });

  it('Stop aborts that agent only', async () => {
    await openHandle();
    withRouter(() => <SubagentTab {...tab('subagent', { agentId: 'ag-1' })} />);
    await waitFor(() => expect(screen.getByTestId('stab-stop')).toBeTruthy());
    screen.getByTestId('stab-stop').click();
    await waitFor(() => expect(world.log.some((l) => l.kind === 'cmd' && l.op === '_oino/agent_abort')).toBe(true));
    const abort = world.log.find((l) => l.kind === 'cmd' && l.op === '_oino/agent_abort')!;
    expect(abort.args).toMatchObject({ agentId: 'ag-1' });
    expect(world.log.some((l) => l.kind === 'cmd' && l.op === 'abort')).toBe(false);
  });
});

describe('phone surfaces (§3.9)', () => {
  beforeEach(() => setDesktopOverride(false));

  it('opens the diff viewer as a full-screen layer and the rest as a sheet', async () => {
    await import('./DiffTab');
    await import('./TreeTab');
    await import('./BtwTab');
    await import('./SubagentTab');
    withRouter(() => <SessionTabLayer />);
    // Diff → layer
    seedPhoneSessionTab({ kind: 'diff', base: { hostId: HOST, sessionId: S }, params: { hostId: HOST, sessionId: S, toolId: 'tc-edit', path: 'src/math.ts' }, title: 'Diff · math.ts' });
    await waitFor(() => expect(screen.getByTestId('session-tab-layer')).toBeTruthy());
    expect(screen.queryByTestId('session-tab-sheet')).toBeNull();
    closePhoneSessionTab();
    await waitFor(() => expect(screen.queryByTestId('session-tab-layer')).toBeNull());
    // Tree → bottom sheet, and it closes
    seedPhoneSessionTab({ kind: 'tree', base: { hostId: HOST, sessionId: S }, params: { hostId: HOST, sessionId: S }, title: 'Tree' });
    await waitFor(() => expect(screen.getByTestId('session-tab-sheet')).toBeTruthy());
    expect(phoneSessionTab()?.kind).toBe('tree');
    screen.getByTestId('session-tab-close').click();
    await waitFor(() => expect(phoneSessionTab()).toBeUndefined());
  });

  it('a phone opener never touches the desktop pane store', async () => {
    openTreeTab({ hostId: HOST, sessionId: S, cwd: CWD });
    expect(phoneSessionTab()?.kind).toBe('tree');
    expect(panes().visible(SCOPE, 'side', { sessionId: S })).toHaveLength(0);
    closePhoneSessionTab();
  });
});
