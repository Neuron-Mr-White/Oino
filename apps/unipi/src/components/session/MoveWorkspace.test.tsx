// @vitest-environment jsdom
/**
 * "No workspace" = a private scratch folder (UniPi Oino), and "Move to a workspace…":
 * the selectors that recognise scratch folders, the draft picker's wording, the
 * timeline's marker row, the header's folder line, the move dialog and the mock
 * host's one-way rule.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { isHomeCwd, isScratchCwd, projectsOf, sessionTitle } from '../../lib/session/selectors';
import { buildTurns, cwdChangeTarget, isCwdChange, type TurnsInput } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import type { SessionCard } from '../../lib/host/client';
import { createMockHost } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { hubs, resetHubs } from '../../lib/session/hub';
import { setDesktopOverride } from '../../lib/layout';
import { CardRow } from '../timeline/cards/CardRow';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const SCRATCH = '/home/you/.local/share/oino/scratch/0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b';
const card = (id: string, cwd: string): SessionCard => ({ session_id: id, name: null, cwd, run_state: 'idle', updated_at: Date.now(), message_count: 2 });

afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});
beforeEach(() => setDesktopOverride(true));

describe('scratch folders are "No workspace"', () => {
  it('recognises the per-OS scratch folder shapes', () => {
    expect(isScratchCwd(SCRATCH)).toBe(true);
    expect(isScratchCwd('/Users/you/Library/Application Support/oino/scratch/abc')).toBe(true);
    expect(isScratchCwd('C:\\Users\\you\\AppData\\Local\\oino\\scratch\\abc')).toBe(true);
    expect(isScratchCwd('/home/you/.local/share/oino/scratch')).toBe(false);
    expect(isScratchCwd('/home/you/p/oino')).toBe(false);
    expect(isScratchCwd(undefined)).toBe(false);
    // "No workspace" covers the scratch folder and (older chats) the bare home.
    expect(isHomeCwd(SCRATCH)).toBe(true);
    expect(isHomeCwd('/home/you')).toBe(true);
    expect(isHomeCwd('/home/you/p/oino')).toBe(false);
  });

  it('groups scratch chats under No workspace and a moved chat under its workspace', () => {
    const groups = projectsOf([card('a', SCRATCH), card('b', `${SCRATCH.slice(0, -4)}ffff`), card('c', '/home/you/p/site')]);
    const none = groups.find((g) => g.cwd === '')!;
    expect(none.label).toBe('No workspace');
    expect(none.cards.map((c) => c.session_id).sort()).toEqual(['a', 'b']);
    expect(groups.find((g) => g.cwd === '/home/you/p/site')!.cards.map((c) => c.session_id)).toEqual(['c']);
    // The title of an unnamed scratch chat is not its uuid folder.
    expect(sessionTitle(card('a', SCRATCH))).toBe('New chat');
  });
});

describe('the timeline marks the move', () => {
  const T0 = Date.UTC(2026, 10, 1, 9, 0, 0);
  const entries: SessionEntry[] = [
    { type: 'message', id: 'e1', parentId: null, timestamp: new Date(T0).toISOString(), message: { role: 'user', content: [{ type: 'text', text: 'hi' }], timestamp: T0 } },
    { type: 'message', id: 'e2', parentId: 'e1', timestamp: new Date(T0 + 1000).toISOString(), message: { role: 'assistant', content: [{ type: 'text', text: 'hello' }], stopReason: 'stop', timestamp: T0 + 1000 } },
    {
      type: 'custom_message',
      id: 'e3',
      parentId: 'e2',
      timestamp: new Date(T0 + 2000).toISOString(),
      customType: 'oino.cwd_change',
      content: '<system-reminder>The workspace changed…</system-reminder>',
      display: true,
      details: { cwd: '/home/you/p/site', previousCwd: SCRATCH },
    },
  ];
  const input: TurnsInput = { entries, leafId: 'e3', entriesRev: 1, liveTools: {}, run: { running: false } } as TurnsInput;

  it('the cwd_change entry sits after the answer (never folded away) and names the folder', () => {
    const [turn] = buildTurns(input);
    const seg = turn!.segments[0]!;
    expect(seg.answer?.text).toBe('hello');
    const after = (seg.after ?? []).flatMap((n) => ('entry' in n ? [n.entry as SessionEntry] : 'items' in n ? (n.items as Array<{ entry?: SessionEntry }>).flatMap((i) => (i.entry ? [i.entry] : [])) : []));
    expect(after.some(isCwdChange)).toBe(true);
    expect(cwdChangeTarget(entries[2]!)).toBe('/home/you/p/site');
  });

  it('renders a "Moved to workspace" marker row, not the reminder text', async () => {
    const ctx = { hostId: 'h', sessionId: () => 's', cwd: () => '/home/you/p/site' } as unknown as Parameters<typeof CardRow>[0]['ctx'];
    render(() => <CardRow entry={entries[2]!} ctx={ctx} />);
    const row = screen.getByTestId('tl-cwd-change');
    expect(row.textContent).toContain('Moved to workspace ~/p/site');
    expect(row.textContent).not.toContain('system-reminder');
  });
});

describe('the draft picker says where No workspace runs', () => {
  it('"Runs in a private scratch folder", and scratch folders never show as recent workspaces', async () => {
    const { WorkspacePicker } = await import('../draft/WorkspacePicker');
    render(() => <WorkspacePicker hostId="h" value={undefined} onChange={() => {}} recent={[SCRATCH, '/home/you/p/site']} roots={['/home/you']} home="/home/you" />);
    const chip = screen.getByTestId('workspace-chip');
    expect(chip.textContent).toContain('No workspace');
    expect(chip.getAttribute('title')).toContain('private scratch folder');
    expect(chip.getAttribute('title')).not.toContain('~');
    fireEvent.click(chip);
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    expect(screen.getByTestId('workspace-none').textContent).toContain('Runs in a private scratch folder');
    expect(menu.textContent).toContain('site');
    expect(menu.textContent).not.toContain('scratch/0199');
  });
});

describe('the header folder line of a scratch chat', () => {
  it('offers Move to a workspace… and Reveal scratch folder; a workspace chat has a plain line', async () => {
    const { ConversationHeader, registerChatAction } = await import('../shell/ConversationHeader');
    const run = vi.fn();
    const off1 = registerChatAction('move-workspace', { run, when: (c) => isScratchCwd(c.cwd) });
    const off2 = registerChatAction('reveal', { run: () => {}, when: (c) => !!c.cwd });
    const ctx = { hostId: 'h', sessionId: 's', title: 'Chat', cwd: SCRATCH };
    const { unmount } = render(() => <ConversationHeader title="Chat" cwd={SCRATCH} project="No workspace · private scratch folder" scratch ctx={ctx} onNewChat={() => {}} desktop />);
    fireEvent.click(screen.getByTestId('conv-folder'));
    const move = await waitFor(() => screen.getByText('Move to a workspace…'));
    expect(screen.getByText('Reveal scratch folder')).toBeTruthy();
    fireEvent.click(move);
    expect(run).toHaveBeenCalledWith(ctx);
    unmount();
    render(() => <ConversationHeader title="Chat" cwd="/home/you/p/site" project="~/p/site" ctx={{ ...ctx, cwd: '/home/you/p/site' }} onNewChat={() => {}} desktop />);
    expect(screen.queryByTestId('conv-folder')).toBeNull();
    off1();
    off2();
  });
});

describe('Move to a workspace (dialog + host)', () => {
  const boot = () => {
    const world = createMockHost({
      name: 't',
      hosts: [{ id: 'pc', name: 'pc', transport: 'local', conn: { state: 'connected' }, cards: [card('s1', SCRATCH), card('s2', '/home/you/p/other'), card('s3', '/home/you')], fs: { '/home/you': { kind: 'dir', children: { p: { kind: 'dir', children: { site: { kind: 'dir', children: {} } } } } } } }],
    });
    setHostClientFactory(world.client);
    return world;
  };

  it('the mock host moves a scratch chat one way only', async () => {
    boot();
    const { moveToWorkspace } = await import('../../lib/session/actions');
    const hub = hubs().ensure('pc');
    await waitFor(() => expect(hub.state.loaded).toBe(true));
    const done = await moveToWorkspace('pc', 's1', '/home/you/p/site');
    expect(done.cwd).toBe('/home/you/p/site');
    await waitFor(() => expect(hub.state.cards.s1?.cwd).toBe('/home/you/p/site'));
    await expect(moveToWorkspace('pc', 's1', '/home/you/p/other')).rejects.toBeTruthy();
    await expect(moveToWorkspace('pc', 's2', '/home/you/p/site')).rejects.toBeTruthy();
  });

  it('an older chat in the bare home moves one way too, never copying from the home', async () => {
    boot();
    const { moveToWorkspace } = await import('../../lib/session/actions');
    const hub = hubs().ensure('pc');
    await waitFor(() => expect(hub.state.loaded).toBe(true));
    await expect(moveToWorkspace('pc', 's3', '/home/you/p/site', { copyFiles: true })).rejects.toBeTruthy();
    await expect(moveToWorkspace('pc', 's3', '/home/you')).rejects.toBeTruthy();
    const done = await moveToWorkspace('pc', 's3', '/home/you/p/site');
    expect(done.cwd).toBe('/home/you/p/site');
    await expect(moveToWorkspace('pc', 's3', '/home/you/p/other')).rejects.toBeTruthy();
  });

  it('the dialog for a home chat disables the copy and never sends copyFiles', async () => {
    const world = boot();
    const requests: Array<[string, Record<string, unknown> | undefined]> = [];
    const client = world.client('pc');
    const request = client.request.bind(client);
    client.request = ((op: string, args?: Record<string, unknown>) => {
      requests.push([op, args]);
      return request(op, args);
    }) as typeof client.request;
    setHostClientFactory(() => client);
    const { MoveWorkspaceDialog, openMoveWorkspaceDialog, moveWorkspaceDialog } = await import('./MoveWorkspaceDialog');
    render(() => <MoveWorkspaceDialog />);
    openMoveWorkspaceDialog({ hostId: 'pc', sessionId: 's3', cwd: '/home/you' });
    await waitFor(() => screen.getByTestId('move-ws-dialog'));
    expect(screen.getByTestId('move-ws-text').textContent).toMatch(/home folder/i);
    expect(screen.getByTestId('move-ws-warning').textContent).toMatch(/home folder is left as it is/i);
    const box = screen.getByTestId('move-ws-copy') as HTMLInputElement;
    expect(box.disabled).toBe(true);
    expect(box.checked).toBe(false);
    await waitFor(() => expect(hubs().ensure('pc').state.loaded).toBe(true));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    fireEvent.click([...menu.querySelectorAll('.ws-row')].find((r) => r.textContent?.includes('other'))!);
    fireEvent.click(screen.getByTestId('move-ws-confirm'));
    await waitFor(() => expect(moveWorkspaceDialog()).toBeUndefined());
    const move = requests.find(([op]) => op === 'session_move');
    expect(move?.[1]).toEqual({ session: 's3', cwd: '/home/you/p/other', copyFiles: false });
  });

  it('the dialog explains it is one way, copy is off by default, Move sends session_move', async () => {
    const world = boot();
    const requests: Array<[string, Record<string, unknown> | undefined]> = [];
    const client = world.client('pc');
    const request = client.request.bind(client);
    client.request = ((op: string, args?: Record<string, unknown>) => {
      requests.push([op, args]);
      return request(op, args);
    }) as typeof client.request;
    setHostClientFactory(() => client);
    const { MoveWorkspaceDialog, openMoveWorkspaceDialog, moveWorkspaceDialog } = await import('./MoveWorkspaceDialog');
    render(() => <MoveWorkspaceDialog />);
    openMoveWorkspaceDialog({ hostId: 'pc', sessionId: 's1', cwd: SCRATCH });
    await waitFor(() => screen.getByTestId('move-ws-dialog'));
    expect(screen.getByTestId('move-ws-warning').textContent).toMatch(/one way/i);
    expect(screen.getByTestId('move-ws-warning').textContent).toMatch(/scratch folder is deleted/i);
    expect((screen.getByTestId('move-ws-copy') as HTMLInputElement).checked).toBe(false);
    expect((screen.getByTestId('move-ws-copy') as HTMLInputElement).disabled).toBe(false);
    expect((screen.getByTestId('move-ws-confirm') as HTMLButtonElement).disabled).toBe(true);
    // Pick a recent workspace from the picker.
    await waitFor(() => expect(hubs().ensure('pc').state.loaded).toBe(true));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    expect(screen.queryByTestId('workspace-none')).toBeNull();
    fireEvent.click([...menu.querySelectorAll('.ws-row')].find((r) => r.textContent?.includes('other'))!);
    fireEvent.click(screen.getByTestId('move-ws-copy'));
    fireEvent.click(screen.getByTestId('move-ws-confirm'));
    await waitFor(() => expect(moveWorkspaceDialog()).toBeUndefined());
    const move = requests.find(([op]) => op === 'session_move');
    expect(move?.[1]).toEqual({ session: 's1', cwd: '/home/you/p/other', copyFiles: true });
  });
});
