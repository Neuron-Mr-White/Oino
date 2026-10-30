// @vitest-environment jsdom
/**
 * UNI-260: New chat sheet (PC → where → folder → first message → start) and
 * Close chat sheet (stop + close tab / just hide); the pure helpers behind them.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { TreeData } from '@unipi/protocol';
import type { ChatListData } from '../../lib/chat/transport';

const call = vi.fn();
vi.mock('../../lib/ipc', () => ({ call: (...a: unknown[]) => call(...a) }));

import { NewChatSheet } from './NewChatSheet';
import { CloseChatSheet } from './CloseChatSheet';
import { folderChoices, folderName, hereCwd, hereTarget, newChatHere, hideChatFromList, hideKey, isOlderHost, pruneHidden, readHidden, shortPath, targetFromNew, unhideChat, whereOptions } from '../../lib/chat/newChat';
import { chatGroups, type ChatTarget } from '../../lib/chat/list';

const pane = (id: string, cwd: string, pi = false, focused = false) => ({ id, focused, cwd, ...(pi ? { agent: { name: 'pi', status: 'idle' as const } } : {}) });
const tree: TreeData = {
  protected: ['guarded'],
  herdr: {
    available: true,
    sessions: [
      {
        name: 'default',
        running: true,
        default: true,
        workspaces: [
          { id: 'w1', label: 'unipi', number: 1, focused: true, agent_status: 'idle', tab_count: 1, pane_count: 1, tabs: [{ id: 'w1:t1', label: '1', number: 1, focused: true, agent_status: 'idle', panes: [pane('w1:p1', '/home/u/unipi', true, true)] }] },
          { id: 'w2', label: 'api', number: 2, focused: false, agent_status: 'idle', tab_count: 1, pane_count: 1, tabs: [{ id: 'w2:t1', label: '1', number: 1, focused: true, agent_status: 'idle', panes: [pane('w2:p1', '/home/u/api')] }] },
        ],
      },
      { name: 'guarded', running: true, default: false, workspaces: [] },
    ],
  },
  tmux: { available: true, sessions: [{ id: '$0', name: 'work', attached: 1, window_count: 1, created: 0, activity: 0, windows: [{ id: '@0', index: 0, name: 'sh', active: true, panes: [{ id: '%0', index: 0, active: true, current_command: 'bash', cwd: '/srv/x', pid: 5, title: '' }] }] }] },
} as TreeData;

function mockHost(opts: { config?: unknown; tree?: TreeData; newChat?: unknown; older?: boolean } = {}) {
  call.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (cmd === 'chat_config') return opts.older ? { ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `chat_config`', retryable: false } } : { ok: true, value: opts.config ?? { launcher: 'pi', default: true, herdr: true, tmux: false } };
    if (cmd === 'mux_tree') return { ok: true, value: { tree: opts.tree ?? tree, path: 'lan' } };
    if (cmd === 'fs_roots') return { ok: true, value: { roots: ['/home/u', '/tmp'] } };
    if (cmd === 'fs_list') return { ok: true, value: { entries: [{ name: 'proj', kind: 'dir', size: 0, mtime: 0, ignored: false }, { name: '.git', kind: 'dir', size: 0, mtime: 0, ignored: false }, { name: 'a.txt', kind: 'file', size: 1, mtime: 0, ignored: false }], more: false } };
    if (cmd === 'chat_new_session') return opts.newChat ?? { ok: true, value: { pid: 4242, mux: 'herdr', session: 'default', workspace_id: 'w9', tab_id: 'w9:t1', pane_id: 'w9:p1', cwd: args.cwd } };
    if (cmd === 'chat_stop') return { ok: true, value: { stopped: true, closed: 'tab' } };
    return { ok: true, value: null };
  });
}

beforeEach(() => {
  call.mockReset();
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

describe('whereOptions / folderChoices / helpers', () => {
  it('lists workspaces (new tab), a new workspace per session, tmux only when available; skips protected', () => {
    const herdrOnly = whereOptions(tree, { herdr: true, tmux: false });
    expect(herdrOnly.map((o) => o.id)).toEqual(['hw:default:w1', 'hw:default:w2', 'hn:default']);
    expect(herdrOnly[0]).toMatchObject({ title: 'unipi', cwd: '/home/u/unipi', needsName: false, target: { kind: 'herdr_workspace', session: 'default', workspace_id: 'w1' } });
    const both = whereOptions(tree, { herdr: true, tmux: true });
    expect(both.map((o) => o.id)).toEqual(['hw:default:w1', 'hw:default:w2', 'hn:default', 'tw:work', 'tn']);
    const tmuxOnly = whereOptions({ ...tree, herdr: { available: false, reason: 'x' } } as TreeData, { herdr: false, tmux: true });
    expect(tmuxOnly.map((o) => o.target.kind)).toEqual(['tmux_window', 'tmux_new_session']);
    expect(whereOptions(undefined, { herdr: true, tmux: true }).map((o) => o.id)).toEqual(['tn']);
  });
  it('folder chips: pi cwds first, then panes, then roots; distinct', () => {
    expect(folderChoices(tree, ['/home/u', '/tmp'])).toEqual(['/home/u/unipi', '/home/u/api', '/srv/x', '/home/u', '/tmp']);
    expect(folderChoices(undefined, ['/home/u'])).toEqual(['/home/u']);
  });
  it('names and paths', () => {
    expect(folderName('/home/u/proj/')).toBe('proj');
    expect(folderName('/')).toBe('pi');
    expect(shortPath('/home/u/proj', '/home/u')).toBe('~/proj');
    expect(shortPath('/srv/x', '/home/u')).toBe('/srv/x');
    expect(isOlderHost({ code: 'E_SCHEMA', message: 'unknown variant `chat_new`' } as never)).toBe(true);
    expect(isOlderHost({ code: 'E_TIMEOUT', message: 'x' } as never)).toBe(false);
  });
  it('targetFromNew maps herdr and tmux', () => {
    const t = targetFromNew('h1', { pid: 5, mux: 'tmux', session: 'work', workspace_id: '@3', pane_id: '%7', cwd: '/home/u/x' }, 'x', 'work');
    expect(t).toMatchObject({ hostId: 'h1', pid: 5, mux: 'tmux', paneId: '%7', workspaceId: '@3', workspaceLabel: 'work', tabId: '' });
  });
  it('hide per PC, unhide, prune', () => {
    const t = { paneId: 'w1:p1', pid: 10 };
    hideChatFromList('h1', t);
    hideChatFromList('h1', t);
    expect(readHidden('h1')).toEqual([hideKey(t)]);
    expect(readHidden('h2')).toEqual([]);
    unhideChat('h1', hideKey(t));
    expect(readHidden('h1')).toEqual([]);
    hideChatFromList('h1', t);
    hideChatFromList('h1', { paneId: 'w1:p2', pid: 11 });
    pruneHidden('h1', [t]);
    expect(readHidden('h1')).toEqual([hideKey(t)]);
  });
  it('chatGroups lists tmux pis from chat_list (UNI-260)', () => {
    const groups = chatGroups('h1', {
      herdr: { available: true, sessions: [] },
      tmux: { available: true, sessions: [{ id: '$1', name: 'work', windows: [{ id: '@2', index: 0, name: 'pi', active: true, panes: [{ id: '%4', index: 0, active: true, cwd: '/home/u/x', pid: 9, title: '', chat: { pid: 77, session_name: 'Fix it', cwd: '/home/u/x' } }, { id: '%5', index: 1, active: false, cwd: '/tmp', pid: 10, title: '' }] }] }] },
    });
    expect(groups).toHaveLength(1);
    expect(groups[0]!.items).toHaveLength(1);
    expect(groups[0]!.items[0]).toMatchObject({ pid: 77, title: 'Fix it', mux: 'tmux', paneId: '%4', workspaceId: '@2' });
  });
});

describe('NewChatSheet', () => {
  const pcs = [{ id: 'h1', name: 'Laptop' }];

  it('defaults to a new workspace in the most-used folder and starts the chat with the first message', async () => {
    mockHost();
    const onStarted = vi.fn();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={onStarted} />);
    await waitFor(() => expect(screen.getByText('New workspace')).toBeTruthy());
    expect(screen.getByRole('radio', { name: /New workspace/ }).getAttribute('aria-checked')).toBe('true');
    // Folder chips + the name follows the folder.
    const chip = screen.getByRole('button', { name: /~\/api|\/home\/u\/api/ });
    fireEvent.click(chip);
    expect((screen.getByLabelText('Workspace name') as HTMLInputElement).value).toBe('api');
    expect(screen.getByText(/Runs/).textContent).toContain('pi');
    fireEvent.input(screen.getByLabelText(/First message/), { target: { value: 'fix the tests' } });
    fireEvent.click(screen.getByTestId('new-chat-start'));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(call).toHaveBeenCalledWith('chat_new_session', { hostId: 'h1', target: { kind: 'herdr_new_workspace', session: 'default', label: 'api' }, cwd: '/home/u/api' }, expect.anything());
    expect(onStarted.mock.calls[0]![0]).toMatchObject({ hostId: 'h1', pcName: 'Laptop', first: 'fix the tests', label: 'api', title: 'api', data: { pid: 4242 } });
  });

  it('an existing workspace opens a tab in its folder; name not asked', async () => {
    mockHost();
    const onStarted = vi.fn();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={onStarted} />);
    await waitFor(() => expect(screen.getByText('unipi')).toBeTruthy());
    fireEvent.click(screen.getByRole('radio', { name: /unipi/ }));
    expect(screen.queryByLabelText('Workspace name')).toBeNull();
    expect(screen.getByText(/Opens a new tab in/).textContent).toContain('unipi');
    fireEvent.click(screen.getByTestId('new-chat-start'));
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(call).toHaveBeenCalledWith('chat_new_session', { hostId: 'h1', target: { kind: 'herdr_workspace', session: 'default', workspace_id: 'w1' }, cwd: '/home/u/unipi' }, expect.anything());
  });

  it('a new workspace needs a name; browsing picks a folder (dirs only, no dotfiles)', async () => {
    mockHost();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={() => {}} />);
    await waitFor(() => expect(screen.getByLabelText('Workspace name')).toBeTruthy());
    fireEvent.input(screen.getByLabelText('Workspace name'), { target: { value: '  ' } });
    expect((screen.getByTestId('new-chat-start') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByText('Browse…'));
    await waitFor(() => expect(screen.getByText('proj')).toBeTruthy());
    expect(screen.queryByText('.git')).toBeNull();
    expect(screen.queryByText('a.txt')).toBeNull();
    fireEvent.click(screen.getByText('proj'));
    await waitFor(() => expect(call).toHaveBeenCalledWith('fs_list', expect.objectContaining({ path: expect.stringMatching(/\/proj$/) }), expect.anything()));
    fireEvent.click(screen.getByTestId('new-chat-use-folder'));
    await waitFor(() => expect(screen.getByRole('button', { name: /proj$/ }).getAttribute('aria-pressed')).toBe('true'));
  });

  it('shows the host error and keeps the form', async () => {
    mockHost({ newChat: { ok: false, error: { code: 'E_CHAT_UNAVAILABLE', message: '`pi` started in a new tab but no pi chat appeared', hint: 'Check the launcher in Settings → PCs.', retryable: true } } });
    const onStarted = vi.fn();
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={onStarted} />);
    await waitFor(() => expect(screen.getByTestId('new-chat-start')).toBeTruthy());
    await waitFor(() => expect((screen.getByTestId('new-chat-start') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId('new-chat-start'));
    await waitFor(() => expect(screen.getByText(/no pi chat appeared/)).toBeTruthy());
    expect(screen.getByText(/Check the launcher/)).toBeTruthy();
    expect(onStarted).not.toHaveBeenCalled();
  });

  it('older host: explains and disables Start', async () => {
    mockHost({ older: true });
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={() => {}} />);
    await waitFor(() => expect(screen.getByText(/older UniPi/)).toBeTruthy());
    expect((screen.getByTestId('new-chat-start') as HTMLButtonElement).disabled).toBe(true);
  });

  it('no herdr and no tmux: says so', async () => {
    mockHost({ config: { launcher: 'pi', default: true, herdr: false, tmux: false }, tree: { herdr: { available: false, reason: 'not installed' }, tmux: { available: false, reason: 'x' } } as TreeData });
    render(() => <NewChatSheet open onClose={() => {}} pcs={pcs} hostId="h1" onStarted={() => {}} />);
    await waitFor(() => expect(screen.getByText(/Neither herdr nor tmux/)).toBeTruthy());
  });

  it('several PCs: asks which first', async () => {
    mockHost();
    render(() => <NewChatSheet open onClose={() => {}} pcs={[...pcs, { id: 'h2', name: 'Desk' }]} onStarted={() => {}} />);
    expect(screen.getByText('Which PC should run it?')).toBeTruthy();
    fireEvent.click(screen.getByText('Desk'));
    await waitFor(() => expect(screen.getByText('Change')).toBeTruthy());
    expect(call).toHaveBeenCalledWith('chat_config', { hostId: 'h2' }, expect.anything());
  });
});

describe('CloseChatSheet', () => {
  const target: ChatTarget = { hostId: 'h1', session: 'default', workspaceId: 'w1', workspaceLabel: 'unipi', tabId: 'w1:t2', paneId: 'w1:p2', pid: 4242, title: 'Fix it', status: 'idle', focused: false };

  it('stop + close tab calls the host and reports done', async () => {
    mockHost();
    const onDone = vi.fn();
    render(() => <CloseChatSheet target={target} pcName="Laptop" running onClose={() => {}} onDone={onDone} />);
    expect(screen.getByText(/pi is working right now/)).toBeTruthy();
    fireEvent.click(screen.getByTestId('close-chat-confirm'));
    await waitFor(() => expect(onDone).toHaveBeenCalledWith('stop'));
    expect(call).toHaveBeenCalledWith('chat_stop', { hostId: 'h1', pid: 4242, closeTab: true }, expect.anything());
  });

  it('just hide never calls the host', () => {
    mockHost();
    const onDone = vi.fn();
    render(() => <CloseChatSheet target={{ ...target, mux: 'tmux' }} pcName="Laptop" onClose={() => {}} onDone={onDone} />);
    expect(screen.getByTestId('close-chat-confirm').textContent).toBe('Stop pi and close window');
    fireEvent.click(screen.getByRole('radio', { name: /Just hide/ }));
    expect(screen.getByTestId('close-chat-confirm').textContent).toBe('Hide from list');
    fireEvent.click(screen.getByTestId('close-chat-confirm'));
    expect(onDone).toHaveBeenCalledWith('hide');
    expect(call).not.toHaveBeenCalled();
  });

  it('a failed stop stays open with the reason', async () => {
    call.mockResolvedValue({ ok: false, error: { code: 'E_TIMEOUT', message: "pi didn't stop within 12 s", hint: 'It may be finishing a tool call.', retryable: true } });
    const onDone = vi.fn();
    render(() => <CloseChatSheet target={target} pcName="Laptop" onClose={() => {}} onDone={onDone} />);
    fireEvent.click(screen.getByTestId('close-chat-confirm'));
    await waitFor(() => expect(screen.getByText(/didn't stop/)).toBeTruthy());
    expect(onDone).not.toHaveBeenCalled();
  });
});

import { LauncherSheet } from './LauncherSheet';
describe('LauncherSheet (Settings → PCs)', () => {
  it('loads, edits and saves the launcher; empty = default', async () => {
    let stored: string | undefined;
    call.mockImplementation(async (cmd: string, args: { launcher?: string }) => {
      if (cmd !== 'chat_config') return { ok: true, value: null };
      if (args.launcher !== undefined) stored = args.launcher;
      return { ok: true, value: { launcher: stored || 'pi', default: !stored, herdr: true, tmux: false } };
    });
    const onClose = vi.fn();
    render(() => <LauncherSheet pc={{ id: 'h1', name: 'Laptop' }} onClose={onClose} />);
    const input = (await screen.findByLabelText('Launcher command')) as HTMLInputElement;
    await waitFor(() => expect(input.disabled).toBe(false));
    expect(input.value).toBe('');
    expect((screen.getByTestId('launcher-save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.input(input, { target: { value: 'unipi --model x' } });
    fireEvent.click(screen.getByTestId('launcher-save'));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(call).toHaveBeenLastCalledWith('chat_config', { hostId: 'h1', launcher: 'unipi --model x' }, expect.anything());
  });
  it('older host', async () => {
    call.mockResolvedValue({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `chat_config`' } });
    render(() => <LauncherSheet pc={{ id: 'h1', name: 'Laptop' }} onClose={() => {}} />);
    await waitFor(() => expect(screen.getByText(/older UniPi/)).toBeTruthy());
  });
});

describe('new chat beside an existing one (same workspace)', () => {
  beforeEach(() => call.mockReset());
  it('targets the chat\'s own herdr workspace (a new tab), or a new tmux window', () => {
    expect(hereTarget({ session: 'default', workspaceId: 'w1' })).toEqual({ kind: 'herdr_workspace', session: 'default', workspace_id: 'w1' });
    expect(hereTarget({ mux: 'tmux', session: 'work', workspaceId: '@0' })).toEqual({ kind: 'tmux_window', session: 'work' });
  });
  it('passes only real host paths as the folder (the list shows ~/… which the host cannot resolve)', () => {
    expect(hereCwd('/home/u/unipi')).toBe('/home/u/unipi');
    expect(hereCwd('C:\\work\\pitest')).toBe('C:\\work\\pitest');
    expect(hereCwd('~/unipi')).toBeUndefined();
    expect(hereCwd(undefined)).toBeUndefined();
  });
  it('asks the host for a new tab in that workspace and folder', async () => {
    mockHost();
    const res = await newChatHere({ hostId: 'h', session: 'default', workspaceId: 'w1' }, '/home/u/unipi');
    expect(res.ok).toBe(true);
    expect(call).toHaveBeenCalledWith('chat_new_session', { hostId: 'h', target: { kind: 'herdr_workspace', session: 'default', workspace_id: 'w1' }, cwd: '/home/u/unipi' }, expect.anything());
  });
  it('the list keeps the real folder next to the shortened one', () => {
    const piPane = { id: 'w1:p1', focused: true, cwd: '/home/u/unipi', agent: { name: 'pi', status: 'idle' }, chat: { pid: 7, cwd: '/home/u/unipi' } };
    const tab = { id: 'w1:t1', label: '1', number: 1, focused: true, agent_status: 'idle', panes: [piPane] };
    const ws = { id: 'w1', label: 'unipi', number: 1, focused: true, agent_status: 'idle', tab_count: 1, pane_count: 1, tabs: [tab] };
    const listData = { herdr: { available: true, sessions: [{ name: 'default', running: true, default: true, workspaces: [ws] }] } } as unknown as ChatListData;
    const groups = chatGroups('h', listData);
    expect(groups[0]!.items[0]!.cwd).toBe('~/unipi');
    expect(groups[0]!.items[0]!.cwdFull).toBe('/home/u/unipi');
  });
});
